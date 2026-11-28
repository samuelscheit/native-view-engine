//! Zero-dependency Apple `CoreFoundation.framework` raw C FFI declarations and
//! RAII [`CfRef`] reference-counting guards (`CFRetain` / `CFRelease` on `Drop`).

use std::ffi::{CStr, c_char, c_void};
use std::ptr::NonNull;

pub(super) type CfTypeRef = *const c_void;
pub(super) type CfAllocatorRef = *const c_void;
pub(super) type CfStringRef = *const c_void;
pub(super) type CfLocaleRef = *const c_void;
pub(super) type CfNumberRef = *const c_void;
pub(super) type CfBooleanRef = *const c_void;
pub(super) type CfDateRef = *const c_void;
pub(super) type CfNumberFormatterRef = *const c_void;
pub(super) type CfDateFormatterRef = *const c_void;
pub(super) type CfCalendarRef = *const c_void;
pub(super) type CfTimeZoneRef = *const c_void;

pub(super) type CfIndex = isize;
pub(super) type CfAbsoluteTime = f64;
pub(super) type CfTimeInterval = f64;
pub(super) type CfStringEncoding = u32;
pub(super) type CfBoolean = u8;

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct CfRange {
    pub location: CfIndex,
    pub length: CfIndex,
}

pub(super) const CF_STRING_ENCODING_UTF8: CfStringEncoding = 0x0800_0100;

// `CFNumberType` constants from `<CoreFoundation/CFNumber.h>`.
pub(super) const CF_NUMBER_SINT32_TYPE: CfIndex = 3;
pub(super) const CF_NUMBER_SINT64_TYPE: CfIndex = 4;
pub(super) const CF_NUMBER_FLOAT64_TYPE: CfIndex = 6;

// `CFNumberFormatterStyle` constants from `<CoreFoundation/CFNumberFormatter.h>`.
pub(super) const CF_NUMBER_STYLE_DECIMAL: CfIndex = 1;
pub(super) const CF_NUMBER_STYLE_CURRENCY: CfIndex = 2;
pub(super) const CF_NUMBER_STYLE_PERCENT: CfIndex = 3;
pub(super) const CF_NUMBER_STYLE_SCIENTIFIC: CfIndex = 4;

// `CFNumberFormatterRoundingMode` constants from `<CoreFoundation/CFNumberFormatter.h>`.
pub(super) const CF_ROUND_CEILING: i32 = 0;
pub(super) const CF_ROUND_FLOOR: i32 = 1;
pub(super) const CF_ROUND_DOWN: i32 = 2;
pub(super) const CF_ROUND_HALF_EVEN: i32 = 4;
pub(super) const CF_ROUND_HALF_UP: i32 = 6;

// `CFDateFormatterStyle` constants from `<CoreFoundation/CFDateFormatter.h>`.
pub(super) const CF_DATE_STYLE_NONE: CfIndex = 0;
pub(super) const CF_DATE_STYLE_SHORT: CfIndex = 1;
pub(super) const CF_DATE_STYLE_MEDIUM: CfIndex = 2;
pub(super) const CF_DATE_STYLE_LONG: CfIndex = 3;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    pub(super) static kCFAllocatorDefault: CfAllocatorRef;
    pub(super) static kCFBooleanTrue: CfBooleanRef;
    pub(super) static kCFBooleanFalse: CfBooleanRef;

    // `CFLocale` keys (`<CoreFoundation/CFLocale.h>`).
    pub(super) static kCFLocaleDecimalSeparator: CfStringRef;
    pub(super) static kCFLocaleGroupingSeparator: CfStringRef;
    pub(super) static kCFLocaleCurrencySymbol: CfStringRef;
    pub(super) static kCFLocaleCurrencyCode: CfStringRef;
    pub(super) static kCFLocaleMeasurementSystem: CfStringRef;
    pub(super) static kCFLocaleUsesMetricSystem: CfStringRef;
    pub(super) static kCFGregorianCalendar: CfStringRef;

    // `CFNumberFormatter` property keys (`<CoreFoundation/CFNumberFormatter.h>`).
    pub(super) static kCFNumberFormatterUseGroupingSeparator: CfStringRef;
    pub(super) static kCFNumberFormatterMinFractionDigits: CfStringRef;
    pub(super) static kCFNumberFormatterMaxFractionDigits: CfStringRef;
    pub(super) static kCFNumberFormatterRoundingMode: CfStringRef;
    pub(super) static kCFNumberFormatterCurrencyCode: CfStringRef;

    // `CFDateFormatter` property keys (`<CoreFoundation/CFDateFormatter.h>`).
    pub(super) static kCFDateFormatterTimeZone: CfStringRef;
    pub(super) static kCFDateFormatterGregorianStartDate: CfStringRef;

    pub(super) fn CFRetain(cf: CfTypeRef) -> CfTypeRef;
    pub(super) fn CFRelease(cf: CfTypeRef);

    pub(super) fn CFBooleanGetValue(boolean: CfBooleanRef) -> CfBoolean;
    pub(super) fn CFDateCreate(allocator: CfAllocatorRef, at: CfAbsoluteTime) -> CfDateRef;

    pub(super) fn CFNumberCreate(
        allocator: CfAllocatorRef,
        the_type: CfIndex,
        value_ptr: *const c_void,
    ) -> CfNumberRef;

    pub(super) fn CFStringCreateWithBytes(
        alloc: CfAllocatorRef,
        bytes: *const u8,
        num_bytes: CfIndex,
        encoding: CfStringEncoding,
        is_external_representation: CfBoolean,
    ) -> CfStringRef;
    pub(super) fn CFStringGetLength(the_string: CfStringRef) -> CfIndex;
    pub(super) fn CFStringGetCStringPtr(
        the_string: CfStringRef,
        encoding: CfStringEncoding,
    ) -> *const c_char;
    pub(super) fn CFStringGetBytes(
        the_string: CfStringRef,
        range: CfRange,
        encoding: CfStringEncoding,
        loss_byte: u8,
        is_external_representation: CfBoolean,
        buffer: *mut u8,
        max_buf_len: CfIndex,
        used_buf_len: *mut CfIndex,
    ) -> CfIndex;

    pub(super) fn CFLocaleCopyCurrent() -> CfLocaleRef;
    pub(super) fn CFLocaleCreate(
        allocator: CfAllocatorRef,
        locale_identifier: CfStringRef,
    ) -> CfLocaleRef;
    pub(super) fn CFLocaleGetIdentifier(locale: CfLocaleRef) -> CfStringRef;
    pub(super) fn CFLocaleGetValue(locale: CfLocaleRef, key: CfStringRef) -> CfTypeRef;

    pub(super) fn CFNumberFormatterCreate(
        allocator: CfAllocatorRef,
        locale: CfLocaleRef,
        style: CfIndex,
    ) -> CfNumberFormatterRef;
    pub(super) fn CFNumberFormatterSetProperty(
        formatter: CfNumberFormatterRef,
        key: CfStringRef,
        value: CfTypeRef,
    );
    pub(super) fn CFNumberFormatterCreateStringWithValue(
        allocator: CfAllocatorRef,
        formatter: CfNumberFormatterRef,
        number_type: CfIndex,
        value_ptr: *const c_void,
    ) -> CfStringRef;

    pub(super) fn CFTimeZoneCreateWithTimeIntervalFromGMT(
        allocator: CfAllocatorRef,
        ti: CfTimeInterval,
    ) -> CfTimeZoneRef;

    pub(super) fn CFDateFormatterCreate(
        allocator: CfAllocatorRef,
        locale: CfLocaleRef,
        date_style: CfIndex,
        time_style: CfIndex,
    ) -> CfDateFormatterRef;
    pub(super) fn CFDateFormatterSetProperty(
        formatter: CfDateFormatterRef,
        key: CfStringRef,
        value: CfTypeRef,
    );
    pub(super) fn CFDateFormatterCreateStringWithAbsoluteTime(
        allocator: CfAllocatorRef,
        formatter: CfDateFormatterRef,
        at: CfAbsoluteTime,
    ) -> CfStringRef;

    pub(super) fn CFCalendarCreateWithIdentifier(
        allocator: CfAllocatorRef,
        identifier: CfStringRef,
    ) -> CfCalendarRef;
    pub(super) fn CFCalendarSetLocale(calendar: CfCalendarRef, locale: CfLocaleRef);
    pub(super) fn CFCalendarGetFirstWeekday(calendar: CfCalendarRef) -> CfIndex;
}

/// RAII owned CoreFoundation reference guard (`CFRetain` on clone/borrow-upgrade,
/// `CFRelease` on `Drop`). Guarantees zero leaks across early returns and panics,
/// and uses [`NonNull`] for null-pointer niche optimization in `Option<CfRef>`.
pub(super) struct CfRef(NonNull<c_void>);

const _: () = assert!(std::mem::size_of::<Option<CfRef>>() == std::mem::size_of::<CfRef>());

impl CfRef {
    /// Wraps an already-owned (`+1` retain count, Create/Copy rule) CF pointer.
    ///
    /// # Safety
    /// If `ptr` is non-null, it must be a valid CoreFoundation object pointer carrying
    /// an owned `+1` retain count transferred to this [`CfRef`] (released on `Drop`).
    #[inline]
    pub(super) unsafe fn from_create(ptr: CfTypeRef) -> Option<Self> {
        NonNull::new(ptr.cast_mut()).map(Self)
    }

    /// Retains a borrowed (Get rule) CF pointer into an owned RAII [`CfRef`].
    ///
    /// # Safety
    /// If `ptr` is non-null, it must be a valid borrowed CoreFoundation object pointer
    /// live for the duration of this call.
    #[inline]
    pub(super) unsafe fn retain_borrowed(ptr: CfTypeRef) -> Option<Self> {
        if ptr.is_null() {
            return None;
        }
        // SAFETY: Caller guarantees `ptr` is a valid non-null borrowed CoreFoundation object;
        // `CFRetain` increments its reference count by 1 so `Drop` releases our balanced retain.
        let retained = unsafe { CFRetain(ptr) };
        // SAFETY: `CFRetain` returns an owned `+1` retain of `ptr`.
        unsafe { Self::from_create(retained) }
    }

    /// Returns the raw CoreFoundation object pointer for FFI calls.
    #[inline]
    pub(super) fn as_ptr(&self) -> CfTypeRef {
        self.0.as_ptr().cast_const()
    }
}

impl Clone for CfRef {
    #[inline]
    fn clone(&self) -> Self {
        // SAFETY: `self.0` is always a valid non-null owned CFTypeRef while `self` is alive.
        let retained = unsafe { CFRetain(self.as_ptr()) };
        Self(
            NonNull::new(retained.cast_mut())
                .expect("CFRetain returned null for live CFTypeRef"),
        )
    }
}

impl Drop for CfRef {
    #[inline]
    fn drop(&mut self) {
        // SAFETY: `self.0` holds a balanced `+1` CoreFoundation retain owned exclusively by `self`.
        unsafe { CFRelease(self.as_ptr()) };
    }
}

/// Creates an owned `CFStringRef` from UTF-8 bytes.
#[inline]
pub(super) fn cf_string_from_bytes(bytes: &[u8]) -> Option<CfRef> {
    let len = CfIndex::try_from(bytes.len()).ok()?;
    // SAFETY: `bytes.as_ptr()` is valid UTF-8 for `len` bytes; CoreFoundation copies the buffer
    // and returns an owned `+1` `CFStringRef`.
    unsafe {
        CfRef::from_create(CFStringCreateWithBytes(
            kCFAllocatorDefault,
            bytes.as_ptr(),
            len,
            CF_STRING_ENCODING_UTF8,
            0,
        ))
    }
}

/// Creates an owned `CFNumberRef` wrapping an `i32`.
#[inline]
pub(super) fn cf_number_i32(val: i32) -> Option<CfRef> {
    // SAFETY: `&val` points to a valid `i32` matching `kCFNumberSInt32Type`;
    // `CFNumberCreate` returns an owned `+1` `CFNumberRef`.
    unsafe {
        CfRef::from_create(CFNumberCreate(
            kCFAllocatorDefault,
            CF_NUMBER_SINT32_TYPE,
            (&raw const val).cast::<c_void>(),
        ))
    }
}

/// Creates an owned `CFDateRef` for `at` seconds since Apple reference epoch (`2001-01-01`).
#[inline]
pub(super) fn cf_date_from_absolute_time(at: CfAbsoluteTime) -> Option<CfRef> {
    // SAFETY: `CFDateCreate` takes any `CFAbsoluteTime` and returns an owned `+1` `CFDateRef`.
    unsafe { CfRef::from_create(CFDateCreate(kCFAllocatorDefault, at)) }
}

/// Appends the UTF-8 contents of `cf_str` directly into `out` without intermediate
/// Rust heap allocations for strings up to 256 UTF-8 bytes.
pub(super) fn append_cf_string(cf_str: &CfRef, out: &mut String) -> bool {
    let cf_str = cf_str.as_ptr();

    // Fast path: direct zero-copy UTF-8 pointer when CoreFoundation stores ASCII/UTF-8 inline.
    // SAFETY: `cf_str` is a valid non-null `CFStringRef`; returned C string pointer (if non-null)
    // is NUL-terminated UTF-8 borrowed for the lifetime of `cf_str`.
    let direct = unsafe { CFStringGetCStringPtr(cf_str, CF_STRING_ENCODING_UTF8) };
    if !direct.is_null() {
        // SAFETY: `CFStringGetCStringPtr` guarantees a valid NUL-terminated string.
        if let Ok(s) = unsafe { CStr::from_ptr(direct) }.to_str() {
            out.push_str(s);
            return true;
        }
    }

    // SAFETY: `cf_str` is a valid non-null `CFStringRef`.
    let utf16_len = unsafe { CFStringGetLength(cf_str) };
    if utf16_len <= 0 {
        return true;
    }

    let range = CfRange {
        location: 0,
        length: utf16_len,
    };

    // Stack fast path: 0 Rust heap allocations for all normal formatted numbers/dates/times.
    let mut stack_buf = [0u8; 256];
    let mut used: CfIndex = 0;
    // SAFETY: `stack_buf` is valid writable memory of 256 bytes; `used` receives bytes written.
    let converted = unsafe {
        CFStringGetBytes(
            cf_str,
            range,
            CF_STRING_ENCODING_UTF8,
            0,
            0,
            stack_buf.as_mut_ptr(),
            stack_buf.len() as CfIndex,
            &raw mut used,
        )
    };
    if converted == utf16_len && used >= 0 && (used as usize) <= stack_buf.len() {
        if let Ok(s) = std::str::from_utf8(&stack_buf[..used as usize]) {
            out.push_str(s);
            return true;
        }
        return false;
    }

    // Long-string fallback (> 256 UTF-8 bytes): query exact byte length, then convert once.
    let mut needed: CfIndex = 0;
    // SAFETY: Passing null buffer queries required UTF-8 byte count into `needed`.
    let queried = unsafe {
        CFStringGetBytes(
            cf_str,
            range,
            CF_STRING_ENCODING_UTF8,
            0,
            0,
            std::ptr::null_mut(),
            0,
            &raw mut needed,
        )
    };
    if queried != utf16_len || needed <= 0 {
        return false;
    }
    let mut heap_buf = vec![0u8; needed as usize];
    let mut written: CfIndex = 0;
    // SAFETY: `heap_buf` has exact length `needed` bytes.
    let ok = unsafe {
        CFStringGetBytes(
            cf_str,
            range,
            CF_STRING_ENCODING_UTF8,
            0,
            0,
            heap_buf.as_mut_ptr(),
            needed,
            &raw mut written,
        )
    };
    if ok != utf16_len || written < 0 || (written as usize) > heap_buf.len() {
        return false;
    }
    if let Ok(s) = std::str::from_utf8(&heap_buf[..written as usize]) {
        out.push_str(s);
        true
    } else {
        false
    }
}
