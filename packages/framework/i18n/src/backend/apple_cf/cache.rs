//! Thread-local bounded FIFO cache for Apple `CoreFoundation` locales,
//! `CFNumberFormatter`, `CFDateFormatter`, and `CFCalendar` week-start metadata.
//!
//! Uses fixed-size inline keys so steady-state lookups perform zero Rust heap
//! allocations, and drops evicted [`CfRef`] entries immediately (`CFRelease`).

use std::cell::RefCell;
use std::fmt;

use crate::format::Weekday;
use crate::locale::LocaleId;

use super::ffi::{
    CF_NUMBER_STYLE_CURRENCY, CFDateFormatterCreate, CFDateFormatterSetProperty, CFLocaleCreate,
    CFNumberFormatterCreate, CFNumberFormatterSetProperty, CFTimeZoneCreateWithTimeIntervalFromGMT,
    CfIndex, CfRef, cf_number_i32, cf_string_from_bytes, kCFAllocatorDefault, kCFBooleanFalse,
    kCFBooleanTrue, kCFDateFormatterTimeZone, kCFNumberFormatterCurrencyCode,
    kCFNumberFormatterMaxFractionDigits, kCFNumberFormatterMinFractionDigits,
    kCFNumberFormatterRoundingMode, kCFNumberFormatterUseGroupingSeparator,
};

const CACHE_CAPACITY: usize = 32;

/// Fixed-capacity stack buffer implementing `fmt::Write` for zero-alloc ICU
/// locale-identifier keys (`lang[_Script][_REGION][@calendar=...;fw=...;hours=...;numbers=...]`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct InlineKey<const N: usize> {
    buf: [u8; N],
    pub(super) len: u8,
}

impl<const N: usize> InlineKey<N> {
    pub(super) const fn empty() -> Self {
        Self {
            buf: [0u8; N],
            len: 0,
        }
    }

    #[inline]
    pub(super) fn as_bytes(&self) -> &[u8] {
        &self.buf[..usize::from(self.len)]
    }

    pub(super) fn from_bytes_upper(src: &[u8]) -> Option<Self> {
        if src.len() > N {
            return None;
        }
        let mut buf = [0u8; N];
        for (i, &b) in src.iter().enumerate() {
            buf[i] = b.to_ascii_uppercase();
        }
        Some(Self {
            buf,
            len: src.len() as u8,
        })
    }
}

impl<const N: usize> fmt::Write for InlineKey<N> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let cur = usize::from(self.len);
        let bytes = s.as_bytes();
        if cur + bytes.len() > N {
            return Err(fmt::Error);
        }
        self.buf[cur..cur + bytes.len()].copy_from_slice(bytes);
        self.len = (cur + bytes.len()) as u8;
        Ok(())
    }
}

pub(super) type IcuLocaleKey = InlineKey<96>;

/// Formats `locale` as an ICU identifier (`@calendar=...;fw=...;hours=...;numbers=...`)
/// into a stack buffer. Using ICU `@key=value` syntax avoids a macOS CoreFoundation
/// quirk where BCP-47 `-u-nu-arab` on `ar-EG` strips the default value and causes
/// `CFNumberFormatterCreate` to return `NULL`.
pub(super) fn icu_locale_key(locale: &LocaleId) -> Option<IcuLocaleKey> {
    let mut key = IcuLocaleKey::empty();
    locale.write_icu_identifier(&mut key).ok()?;
    Some(key)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct NumFmtKey {
    pub(super) locale: IcuLocaleKey,
    pub(super) style: CfIndex,
    pub(super) use_grouping: bool,
    pub(super) min_frac: i8,
    pub(super) max_frac: i8,
    pub(super) rounding: i32,
    pub(super) currency: InlineKey<8>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct DateFmtKey {
    pub(super) locale: IcuLocaleKey,
    pub(super) date_style: CfIndex,
    pub(super) time_style: CfIndex,
}

/// Fixed-capacity FIFO cache with zero allocations after warmup.
pub(super) struct BoundedCache<K, V> {
    entries: Vec<(K, V)>,
    next_evict: usize,
}

impl<K: Eq, V> BoundedCache<K, V> {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
            next_evict: 0,
        }
    }

    #[inline]
    pub(super) fn get(&self, key: &K) -> Option<&V> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub(super) fn insert(&mut self, key: K, value: V) -> &V {
        if let Some(pos) = self.entries.iter().position(|(k, _)| k == &key) {
            self.entries[pos].1 = value;
            return &self.entries[pos].1;
        }
        if self.entries.len() < CACHE_CAPACITY {
            if self.entries.capacity() == 0 {
                self.entries.reserve_exact(CACHE_CAPACITY);
            }
            self.entries.push((key, value));
            let last = self.entries.len() - 1;
            return &self.entries[last].1;
        }
        let slot = self.next_evict % CACHE_CAPACITY;
        self.next_evict = self.next_evict.wrapping_add(1);
        self.entries[slot] = (key, value);
        &self.entries[slot].1
    }
}

pub(super) struct AppleCfCache {
    utc_tz: Option<CfRef>,
    locales: BoundedCache<IcuLocaleKey, CfRef>,
    num_formatters: BoundedCache<NumFmtKey, CfRef>,
    date_formatters: BoundedCache<DateFmtKey, CfRef>,
    pub(super) first_weekdays: BoundedCache<IcuLocaleKey, Weekday>,
}

impl AppleCfCache {
    fn new() -> Self {
        Self {
            utc_tz: None,
            locales: BoundedCache::new(),
            num_formatters: BoundedCache::new(),
            date_formatters: BoundedCache::new(),
            first_weekdays: BoundedCache::new(),
        }
    }

    fn utc_timezone(&mut self) -> Option<CfRef> {
        if let Some(tz) = &self.utc_tz {
            return Some(tz.clone());
        }
        // SAFETY: Passing `kCFAllocatorDefault` and `0.0` GMT offset creates a valid UTC timezone.
        let tz = CfRef::from_create(unsafe {
            CFTimeZoneCreateWithTimeIntervalFromGMT(kCFAllocatorDefault, 0.0)
        })?;
        self.utc_tz = Some(tz.clone());
        Some(tz)
    }

    pub(super) fn get_or_create_locale(&mut self, key: IcuLocaleKey) -> Option<CfRef> {
        if let Some(existing) = self.locales.get(&key) {
            return Some(existing.clone());
        }
        let cf_id = cf_string_from_bytes(key.as_bytes())?;
        // SAFETY: `cf_id` is a valid non-null CFStringRef containing an ICU locale identifier.
        let cf_loc = CfRef::from_create(unsafe {
            CFLocaleCreate(kCFAllocatorDefault, cf_id.as_ptr())
        })?;
        Some(self.locales.insert(key, cf_loc).clone())
    }

    pub(super) fn get_or_create_number_formatter(&mut self, key: NumFmtKey) -> Option<CfRef> {
        if let Some(existing) = self.num_formatters.get(&key) {
            return Some(existing.clone());
        }
        let cf_loc = self.get_or_create_locale(key.locale)?;
        let nf = create_number_formatter(&cf_loc, key)?;
        Some(self.num_formatters.insert(key, nf).clone())
    }

    pub(super) fn get_or_create_date_formatter(&mut self, key: DateFmtKey) -> Option<CfRef> {
        if let Some(existing) = self.date_formatters.get(&key) {
            return Some(existing.clone());
        }
        let cf_loc = self.get_or_create_locale(key.locale)?;
        let utc_tz = self.utc_timezone()?;
        let df = create_date_formatter(&cf_loc, &utc_tz, key.date_style, key.time_style)?;
        Some(self.date_formatters.insert(key, df).clone())
    }
}

thread_local! {
    pub(super) static CF_CACHE: RefCell<AppleCfCache> = RefCell::new(AppleCfCache::new());
}

pub(super) fn create_uncached_locale(locale: &LocaleId) -> Option<CfRef> {
    let mut id_str = String::with_capacity(64);
    locale.write_icu_identifier(&mut id_str).ok()?;
    let cf_id = cf_string_from_bytes(id_str.as_bytes())?;
    // SAFETY: `cf_id` is a valid `CFStringRef`.
    CfRef::from_create(unsafe { CFLocaleCreate(kCFAllocatorDefault, cf_id.as_ptr()) })
}

pub(super) fn create_number_formatter(cf_loc: &CfRef, key: NumFmtKey) -> Option<CfRef> {
    // SAFETY: `cf_loc` is a valid `CFLocaleRef` and `key.style` is a valid `CFNumberFormatterStyle`.
    let nf = CfRef::from_create(unsafe {
        CFNumberFormatterCreate(kCFAllocatorDefault, cf_loc.as_ptr(), key.style)
    })?;

    if key.style == CF_NUMBER_STYLE_CURRENCY && key.currency.len > 0 {
        let cf_cur = cf_string_from_bytes(key.currency.as_bytes())?;
        // SAFETY: `nf` is a valid `CFNumberFormatterRef` and `cf_cur` is a valid `CFStringRef`.
        unsafe {
            CFNumberFormatterSetProperty(nf.as_ptr(), kCFNumberFormatterCurrencyCode, cf_cur.as_ptr());
        }
    }

    // SAFETY: `kCFBooleanTrue`/`kCFBooleanFalse` are CoreFoundation boolean singletons.
    let grp_bool = unsafe {
        if key.use_grouping {
            kCFBooleanTrue
        } else {
            kCFBooleanFalse
        }
    };
    // SAFETY: `nf` is a valid `CFNumberFormatterRef` and `grp_bool` is a valid `CFBooleanRef`.
    unsafe {
        CFNumberFormatterSetProperty(nf.as_ptr(), kCFNumberFormatterUseGroupingSeparator, grp_bool);
    }

    if key.min_frac >= 0 {
        let n = cf_number_i32(i32::from(key.min_frac))?;
        // SAFETY: `nf` is valid and `n` is a valid `CFNumberRef`.
        unsafe {
            CFNumberFormatterSetProperty(nf.as_ptr(), kCFNumberFormatterMinFractionDigits, n.as_ptr());
        }
        if key.max_frac < 0 && key.min_frac > 3 {
            // SAFETY: Keep maxFractionDigits >= minFractionDigits when max wasn't explicitly set.
            unsafe {
                CFNumberFormatterSetProperty(
                    nf.as_ptr(),
                    kCFNumberFormatterMaxFractionDigits,
                    n.as_ptr(),
                );
            }
        }
    }

    if key.max_frac >= 0 {
        let n = cf_number_i32(i32::from(key.max_frac))?;
        // SAFETY: `nf` is valid and `n` is a valid `CFNumberRef`.
        unsafe {
            CFNumberFormatterSetProperty(nf.as_ptr(), kCFNumberFormatterMaxFractionDigits, n.as_ptr());
        }
    }

    let round_num = cf_number_i32(key.rounding)?;
    // SAFETY: `nf` is valid and `round_num` holds a valid `CFNumberFormatterRoundingMode`.
    unsafe {
        CFNumberFormatterSetProperty(nf.as_ptr(), kCFNumberFormatterRoundingMode, round_num.as_ptr());
    }

    Some(nf)
}

pub(super) fn create_date_formatter(
    cf_loc: &CfRef,
    utc_tz: &CfRef,
    date_style: CfIndex,
    time_style: CfIndex,
) -> Option<CfRef> {
    // SAFETY: `cf_loc` is a valid `CFLocaleRef`; `date_style` and `time_style` are valid style enums.
    let df = CfRef::from_create(unsafe {
        CFDateFormatterCreate(kCFAllocatorDefault, cf_loc.as_ptr(), date_style, time_style)
    })?;
    // SAFETY: `df` is a valid `CFDateFormatterRef` and `utc_tz` is a valid `CFTimeZoneRef`.
    unsafe {
        CFDateFormatterSetProperty(df.as_ptr(), kCFDateFormatterTimeZone, utc_tz.as_ptr());
    }
    Some(df)
}
