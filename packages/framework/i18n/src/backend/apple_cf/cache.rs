//! Thread-local bounded FIFO cache for Apple `CoreFoundation` locales,
//! `CFNumberFormatter`, `CFDateFormatter`, and `CFCalendar` week-start metadata.
//!
//! Uses fixed-size inline keys so steady-state lookups perform zero Rust heap
//! allocations and zero `CFRetain`/`CFRelease` traffic on cache hits, and drops
//! evicted [`CfRef`] entries immediately (`CFRelease`).

use std::cell::RefCell;
use std::fmt;

use crate::format::Weekday;
use crate::locale::LocaleId;

use super::ffi::{
    CF_DATE_STYLE_NONE, CF_NUMBER_STYLE_CURRENCY, CF_NUMBER_STYLE_PERCENT, CFDateFormatterCreate,
    CFDateFormatterSetProperty, CFLocaleCreate, CFNumberFormatterCreate,
    CFNumberFormatterSetProperty, CFTimeZoneCreateWithTimeIntervalFromGMT, CfIndex, CfRef,
    cf_date_from_absolute_time, cf_number_i32, cf_string_from_bytes, kCFAllocatorDefault,
    kCFBooleanFalse, kCFBooleanTrue, kCFDateFormatterGregorianStartDate, kCFDateFormatterTimeZone,
    kCFNumberFormatterCurrencyCode, kCFNumberFormatterMaxFractionDigits,
    kCFNumberFormatterMinFractionDigits, kCFNumberFormatterRoundingMode,
    kCFNumberFormatterUseGroupingSeparator,
};

const CACHE_CAPACITY: usize = 32;

/// Distant-past `CFAbsoluteTime` (`-1e17` s ≈ `-3.17e9` years, covering the full `i32`
/// year range while staying finite in ICU's millisecond `UDate`) used to configure
/// `kCFDateFormatterGregorianStartDate` for proleptic Gregorian formatting.
const PROLEPTIC_GREGORIAN_START_AT: f64 = -1.0e17;

/// Fixed-capacity stack buffer implementing `fmt::Write` for zero-alloc ICU
/// locale-identifier keys (`lang[_Script][_REGION][@calendar=...;fw=...;hours=...;numbers=...]`).
#[derive(Clone, Copy, Eq)]
pub(super) struct InlineKey<const N: usize> {
    buf: [u8; N],
    pub(super) len: u8,
}

impl<const N: usize> InlineKey<N> {
    pub(super) const fn empty() -> Self {
        const { assert!(N <= u8::MAX as usize) };
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
        const { assert!(N <= u8::MAX as usize) };
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

impl<const N: usize> PartialEq for InlineKey<N> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.len == other.len && self.as_bytes() == other.as_bytes()
    }
}

impl<const N: usize> fmt::Write for InlineKey<N> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        const { assert!(N <= u8::MAX as usize) };
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
    pub(super) style: CfIndex,
    pub(super) rounding: i32,
    pub(super) min_frac: i8,
    pub(super) max_frac: i8,
    pub(super) use_grouping: bool,
    pub(super) currency: InlineKey<8>,
    pub(super) locale: IcuLocaleKey,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct DateFmtKey {
    pub(super) date_style: CfIndex,
    pub(super) time_style: CfIndex,
    pub(super) proleptic_gregorian: bool,
    pub(super) locale: IcuLocaleKey,
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
    fn position(&self, key: &K) -> Option<usize> {
        self.entries.iter().position(|(k, _)| k == key)
    }

    #[inline]
    pub(super) fn get(&self, key: &K) -> Option<&V> {
        self.position(key).map(|idx| &self.entries[idx].1)
    }

    pub(super) fn insert(&mut self, key: K, value: V) -> &V {
        if let Some(pos) = self.position(&key) {
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

fn create_utc_timezone() -> Option<CfRef> {
    // SAFETY: Passing `kCFAllocatorDefault` and `0.0` GMT offset creates an owned `+1` UTC timezone.
    unsafe { CfRef::from_create(CFTimeZoneCreateWithTimeIntervalFromGMT(kCFAllocatorDefault, 0.0)) }
}

fn get_or_create_utc_tz(slot: &mut Option<CfRef>) -> Option<&CfRef> {
    if slot.is_none() {
        *slot = Some(create_utc_timezone()?);
    }
    slot.as_ref()
}

fn get_or_create_proleptic_start(slot: &mut Option<CfRef>) -> Option<&CfRef> {
    if slot.is_none() {
        *slot = Some(cf_date_from_absolute_time(PROLEPTIC_GREGORIAN_START_AT)?);
    }
    slot.as_ref()
}

fn get_or_create_locale_in(
    locales: &mut BoundedCache<IcuLocaleKey, CfRef>,
    key: IcuLocaleKey,
) -> Option<&CfRef> {
    if let Some(pos) = locales.position(&key) {
        return Some(&locales.entries[pos].1);
    }
    let cf_id = cf_string_from_bytes(key.as_bytes())?;
    // SAFETY: `cf_id` is a valid non-null `CFStringRef` containing an ICU locale identifier;
    // `CFLocaleCreate` returns an owned `+1` `CFLocaleRef` (or NULL).
    let cf_loc =
        unsafe { CfRef::from_create(CFLocaleCreate(kCFAllocatorDefault, cf_id.as_ptr())) }?;
    Some(locales.insert(key, cf_loc))
}

pub(super) struct AppleCfCache {
    utc_tz: Option<CfRef>,
    proleptic_start: Option<CfRef>,
    locales: BoundedCache<IcuLocaleKey, CfRef>,
    num_formatters: BoundedCache<NumFmtKey, CfRef>,
    date_formatters: BoundedCache<DateFmtKey, CfRef>,
    first_weekdays: BoundedCache<IcuLocaleKey, Weekday>,
}

impl AppleCfCache {
    fn new() -> Self {
        Self {
            utc_tz: None,
            proleptic_start: None,
            locales: BoundedCache::new(),
            num_formatters: BoundedCache::new(),
            date_formatters: BoundedCache::new(),
            first_weekdays: BoundedCache::new(),
        }
    }

    fn get_or_create_number_formatter(&mut self, key: NumFmtKey) -> Option<&CfRef> {
        if let Some(pos) = self.num_formatters.position(&key) {
            return Some(&self.num_formatters.entries[pos].1);
        }
        let cf_loc = get_or_create_locale_in(&mut self.locales, key.locale)?;
        let nf = create_number_formatter(cf_loc, key)?;
        Some(self.num_formatters.insert(key, nf))
    }

    fn get_or_create_date_formatter(&mut self, key: DateFmtKey) -> Option<&CfRef> {
        if let Some(pos) = self.date_formatters.position(&key) {
            return Some(&self.date_formatters.entries[pos].1);
        }
        let cf_loc = get_or_create_locale_in(&mut self.locales, key.locale)?;
        let utc_tz = get_or_create_utc_tz(&mut self.utc_tz)?;
        let proleptic = if key.proleptic_gregorian && key.date_style != CF_DATE_STYLE_NONE {
            get_or_create_proleptic_start(&mut self.proleptic_start)
        } else {
            None
        };
        let df = create_date_formatter(cf_loc, utc_tz, proleptic, key.date_style, key.time_style)?;
        Some(self.date_formatters.insert(key, df))
    }
}

thread_local! {
    static CF_CACHE: RefCell<AppleCfCache> = RefCell::new(AppleCfCache::new());
}

fn create_uncached_locale(locale: &LocaleId) -> Option<CfRef> {
    let mut id_str = String::with_capacity(64);
    locale.write_icu_identifier(&mut id_str).ok()?;
    let cf_id = cf_string_from_bytes(id_str.as_bytes())?;
    // SAFETY: `cf_id` is a valid `CFStringRef`; `CFLocaleCreate` returns an owned `+1` ref.
    unsafe { CfRef::from_create(CFLocaleCreate(kCFAllocatorDefault, cf_id.as_ptr())) }
}

/// Runs `f` with a borrowed cached (or uncached fallback) `CFNumberFormatterRef`,
/// avoiding per-hit `CFRetain`/`CFRelease` and degrading cleanly during TLS teardown.
pub(super) fn with_number_formatter(
    locale: &LocaleId,
    key_base: NumFmtKey,
    f: impl FnOnce(&CfRef) -> bool,
) -> bool {
    let mut f = Some(f);
    if let Some(loc_key) = icu_locale_key(locale) {
        let key = NumFmtKey {
            locale: loc_key,
            ..key_base
        };
        if let Ok(ok) = CF_CACHE.try_with(|cell| {
            let mut cache = cell.borrow_mut();
            let Some(nf) = cache.get_or_create_number_formatter(key) else {
                return false;
            };
            f.take().is_some_and(|cb| cb(nf))
        }) {
            return ok;
        }
    }

    let Some(cf_loc) = create_uncached_locale(locale) else {
        return false;
    };
    let Some(nf) = create_number_formatter(&cf_loc, key_base) else {
        return false;
    };
    f.take().is_some_and(|cb| cb(&nf))
}

fn with_date_time_formatter(
    locale: &LocaleId,
    date_style: CfIndex,
    time_style: CfIndex,
    proleptic_gregorian: bool,
    f: impl FnOnce(&CfRef) -> bool,
) -> bool {
    let mut f = Some(f);
    if let Some(loc_key) = icu_locale_key(locale) {
        let key = DateFmtKey {
            date_style,
            time_style,
            proleptic_gregorian,
            locale: loc_key,
        };
        if let Ok(ok) = CF_CACHE.try_with(|cell| {
            let mut cache = cell.borrow_mut();
            let Some(df) = cache.get_or_create_date_formatter(key) else {
                return false;
            };
            f.take().is_some_and(|cb| cb(df))
        }) {
            return ok;
        }
    }

    let Some(cf_loc) = create_uncached_locale(locale) else {
        return false;
    };
    let Some(utc_tz) = create_utc_timezone() else {
        return false;
    };
    let proleptic = if proleptic_gregorian && date_style != CF_DATE_STYLE_NONE {
        cf_date_from_absolute_time(PROLEPTIC_GREGORIAN_START_AT)
    } else {
        None
    };
    let Some(df) = create_date_formatter(&cf_loc, &utc_tz, proleptic.as_ref(), date_style, time_style)
    else {
        return false;
    };
    f.take().is_some_and(|cb| cb(&df))
}

/// Runs `f` with a borrowed cached (or uncached fallback) date-only `CFDateFormatterRef`.
pub(super) fn with_date_formatter(
    locale: &LocaleId,
    date_style: CfIndex,
    f: impl FnOnce(&CfRef) -> bool,
) -> bool {
    let proleptic_gregorian = locale
        .calendar()
        .is_none_or(|c| matches!(c, "gregory" | "gregorian" | "iso8601"));
    with_date_time_formatter(locale, date_style, CF_DATE_STYLE_NONE, proleptic_gregorian, f)
}

/// Runs `f` with a borrowed cached (or uncached fallback) time-only `CFDateFormatterRef`.
pub(super) fn with_time_formatter(
    locale: &LocaleId,
    time_style: CfIndex,
    f: impl FnOnce(&CfRef) -> bool,
) -> bool {
    with_date_time_formatter(locale, CF_DATE_STYLE_NONE, time_style, false, f)
}

/// Runs `f` with a borrowed cached (or uncached fallback) `CFLocaleRef`.
pub(super) fn with_locale<R>(locale: &LocaleId, f: impl FnOnce(&CfRef) -> Option<R>) -> Option<R> {
    let mut f = Some(f);
    if let Some(loc_key) = icu_locale_key(locale)
        && let Ok(res) = CF_CACHE.try_with(|cell| {
            let mut cache = cell.borrow_mut();
            let cf_loc = get_or_create_locale_in(&mut cache.locales, loc_key)?;
            f.take().and_then(|cb| cb(cf_loc))
        })
    {
        return res;
    }
    let cf_loc = create_uncached_locale(locale)?;
    f.take().and_then(|cb| cb(&cf_loc))
}

/// Looks up or computes `locale`'s cached first day of the week via `query`.
pub(super) fn get_or_insert_first_weekday(
    locale: &LocaleId,
    query: impl FnOnce(&CfRef) -> Option<Weekday>,
) -> Option<Weekday> {
    let mut query = Some(query);
    if let Some(loc_key) = icu_locale_key(locale)
        && let Ok(res) = CF_CACHE.try_with(|cell| {
            let mut cache = cell.borrow_mut();
            if let Some(&day) = cache.first_weekdays.get(&loc_key) {
                return Some(day);
            }
            let cf_loc = get_or_create_locale_in(&mut cache.locales, loc_key)?;
            let day = query.take().and_then(|cb| cb(cf_loc))?;
            cache.first_weekdays.insert(loc_key, day);
            Some(day)
        })
    {
        return res;
    }
    let cf_loc = create_uncached_locale(locale)?;
    query.take().and_then(|cb| cb(&cf_loc))
}

fn create_number_formatter(cf_loc: &CfRef, key: NumFmtKey) -> Option<CfRef> {
    // SAFETY: `cf_loc` is a valid `CFLocaleRef` and `key.style` is a valid `CFNumberFormatterStyle`.
    let nf = unsafe {
        CfRef::from_create(CFNumberFormatterCreate(
            kCFAllocatorDefault,
            cf_loc.as_ptr(),
            key.style,
        ))
    }?;

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

    let default_max: i8 = if key.style == CF_NUMBER_STYLE_PERCENT {
        0
    } else {
        3
    };
    let eff_max = if key.max_frac >= 0 {
        let clamped = if key.min_frac >= 0 {
            key.max_frac.max(key.min_frac)
        } else {
            key.max_frac
        };
        Some(clamped)
    } else if key.min_frac > default_max {
        Some(key.min_frac)
    } else {
        None
    };

    // Set MaxFractionDigits before MinFractionDigits so `min <= max` holds at every step.
    if let Some(max_f) = eff_max {
        let n = cf_number_i32(i32::from(max_f))?;
        // SAFETY: `nf` is valid and `n` is a valid `CFNumberRef`.
        unsafe {
            CFNumberFormatterSetProperty(
                nf.as_ptr(),
                kCFNumberFormatterMaxFractionDigits,
                n.as_ptr(),
            );
        }
    }

    if key.min_frac >= 0 {
        let n = cf_number_i32(i32::from(key.min_frac))?;
        // SAFETY: `nf` is valid and `n` is a valid `CFNumberRef`.
        unsafe {
            CFNumberFormatterSetProperty(
                nf.as_ptr(),
                kCFNumberFormatterMinFractionDigits,
                n.as_ptr(),
            );
        }
    }

    let round_num = cf_number_i32(key.rounding)?;
    // SAFETY: `nf` is valid and `round_num` holds a valid `CFNumberFormatterRoundingMode`.
    unsafe {
        CFNumberFormatterSetProperty(nf.as_ptr(), kCFNumberFormatterRoundingMode, round_num.as_ptr());
    }

    Some(nf)
}

fn create_date_formatter(
    cf_loc: &CfRef,
    utc_tz: &CfRef,
    proleptic_start: Option<&CfRef>,
    date_style: CfIndex,
    time_style: CfIndex,
) -> Option<CfRef> {
    // SAFETY: `cf_loc` is a valid `CFLocaleRef`; `date_style` and `time_style` are valid style enums.
    let df = unsafe {
        CfRef::from_create(CFDateFormatterCreate(
            kCFAllocatorDefault,
            cf_loc.as_ptr(),
            date_style,
            time_style,
        ))
    }?;
    // SAFETY: `df` is a valid `CFDateFormatterRef` and `utc_tz` is a valid `CFTimeZoneRef`.
    unsafe {
        CFDateFormatterSetProperty(df.as_ptr(), kCFDateFormatterTimeZone, utc_tz.as_ptr());
    }
    if let Some(start) = proleptic_start {
        // SAFETY: `df` is a valid `CFDateFormatterRef` and `start` is a valid `CFDateRef`.
        unsafe {
            CFDateFormatterSetProperty(
                df.as_ptr(),
                kCFDateFormatterGregorianStartDate,
                start.as_ptr(),
            );
        }
    }
    Some(df)
}
