//! Typed, zero-serialization cross-platform autofill, password manager, OTP SMS, and virtual
//! keyboard input-mode hints (`AutofillHint` and `InputMode`).
//!
//! Maps a single platform-agnostic Rust enum directly to native platform identifiers:
//! - Web HTML5: `<input autocomplete="..." inputmode="...">`
//! - Apple UIKit/AppKit: `UITextContentType` & `UIKeyboardType`
//! - Android: `View.AUTOFILL_HINT_*` / `androidx.autofill.HintConstants` & `InputType`

use core::fmt;

/// Semantic autofill, password manager, and OTP SMS hint for text inputs.
///
/// Maps directly to canonical browser HTML `autocomplete`, Apple `UITextContentType`, and
/// Android `AUTOFILL_HINT_*` identifiers with zero heap allocation or JSON serialization.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, nve_store::WordEnum)]
pub enum AutofillHint {
    Off = 0,
    Name = 1,
    GivenName = 2,
    FamilyName = 3,
    Email = 4,
    Username = 5,
    CurrentPassword = 6,
    NewPassword = 7,
    OneTimeCode = 8,
    Tel = 9,
    StreetAddress = 10,
    PostalCode = 11,
    Country = 12,
    CcNumber = 13,
    CcExp = 14,
    CcCsc = 15,
}

impl AutofillHint {

    /// Canonical WHATWG HTML `<input autocomplete="...">` token.
    #[inline]
    pub const fn whatwg_token(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Name => "name",
            Self::GivenName => "given-name",
            Self::FamilyName => "family-name",
            Self::Email => "email",
            Self::Username => "username",
            Self::CurrentPassword => "current-password",
            Self::NewPassword => "new-password",
            Self::OneTimeCode => "one-time-code",
            Self::Tel => "tel",
            Self::StreetAddress => "street-address",
            Self::PostalCode => "postal-code",
            Self::Country => "country",
            Self::CcNumber => "cc-number",
            Self::CcExp => "cc-exp",
            Self::CcCsc => "cc-csc",
        }
    }

    /// Parses a canonical HTML `autocomplete` token (case-insensitive, trimmed) back into
    /// [`AutofillHint`].
    pub fn from_whatwg_token(raw: &str) -> Option<Self> {
        let s = raw.trim();
        if s.eq_ignore_ascii_case("off") {
            Some(Self::Off)
        } else if s.eq_ignore_ascii_case("name") {
            Some(Self::Name)
        } else if s.eq_ignore_ascii_case("given-name") {
            Some(Self::GivenName)
        } else if s.eq_ignore_ascii_case("family-name") {
            Some(Self::FamilyName)
        } else if s.eq_ignore_ascii_case("email") {
            Some(Self::Email)
        } else if s.eq_ignore_ascii_case("username") {
            Some(Self::Username)
        } else if s.eq_ignore_ascii_case("current-password") {
            Some(Self::CurrentPassword)
        } else if s.eq_ignore_ascii_case("new-password") {
            Some(Self::NewPassword)
        } else if s.eq_ignore_ascii_case("one-time-code") {
            Some(Self::OneTimeCode)
        } else if s.eq_ignore_ascii_case("tel") {
            Some(Self::Tel)
        } else if s.eq_ignore_ascii_case("street-address") {
            Some(Self::StreetAddress)
        } else if s.eq_ignore_ascii_case("postal-code") {
            Some(Self::PostalCode)
        } else if s.eq_ignore_ascii_case("country") {
            Some(Self::Country)
        } else if s.eq_ignore_ascii_case("cc-number") {
            Some(Self::CcNumber)
        } else if s.eq_ignore_ascii_case("cc-exp") {
            Some(Self::CcExp)
        } else if s.eq_ignore_ascii_case("cc-csc") {
            Some(Self::CcCsc)
        } else {
            None
        }
    }

    /// Canonical Apple `UITextContentType` raw identifier (`None` for [`AutofillHint::Off`], where
    /// UIKit disables autofill by clearing `textContentType`).
    #[inline]
    pub const fn as_apple_content_type(self) -> Option<&'static str> {
        match self {
            Self::Off => None,
            Self::Name => Some("name"),
            Self::GivenName => Some("givenName"),
            Self::FamilyName => Some("familyName"),
            Self::Email => Some("emailAddress"),
            Self::Username => Some("username"),
            Self::CurrentPassword => Some("password"),
            Self::NewPassword => Some("newPassword"),
            Self::OneTimeCode => Some("oneTimeCode"),
            Self::Tel => Some("telephoneNumber"),
            Self::StreetAddress => Some("fullStreetAddress"),
            Self::PostalCode => Some("postalCode"),
            Self::Country => Some("countryName"),
            Self::CcNumber => Some("creditCardNumber"),
            Self::CcExp => Some("creditCardExpiration"),
            Self::CcCsc => Some("creditCardSecurityCode"),
        }
    }

    /// Canonical Android `View.AUTOFILL_HINT_*` / `androidx.autofill.HintConstants` string
    /// (`None` for [`AutofillHint::Off`], where Android sets `IMPORTANT_FOR_AUTOFILL_NO`).
    #[inline]
    pub const fn as_android_autofill_hint(self) -> Option<&'static str> {
        match self {
            Self::Off => None,
            Self::Name => Some("personName"),
            Self::GivenName => Some("personGivenName"),
            Self::FamilyName => Some("personFamilyName"),
            Self::Email => Some("emailAddress"),
            Self::Username => Some("username"),
            Self::CurrentPassword => Some("password"),
            Self::NewPassword => Some("newPassword"),
            Self::OneTimeCode => Some("smsOTPCode"),
            Self::Tel => Some("phoneNumber"),
            Self::StreetAddress => Some("streetAddress"),
            Self::PostalCode => Some("postalCode"),
            Self::Country => Some("addressCountry"),
            Self::CcNumber => Some("creditCardNumber"),
            Self::CcExp => Some("creditCardExpirationDate"),
            Self::CcCsc => Some("creditCardSecurityCode"),
        }
    }
}

impl fmt::Display for AutofillHint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.whatwg_token())
    }
}

/// Virtual keyboard layout hint for text inputs (WHATWG HTML `inputmode` + native OS keyboards).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, nve_store::WordEnum)]
pub enum InputMode {
    #[default]
    Text = 0,
    Email = 1,
    Tel = 2,
    Url = 3,
    Numeric = 4,
    Decimal = 5,
    Search = 6,
}

impl InputMode {

    /// Canonical WHATWG HTML `<input inputmode="...">` token.
    #[inline]
    pub const fn whatwg_token(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Email => "email",
            Self::Tel => "tel",
            Self::Url => "url",
            Self::Numeric => "numeric",
            Self::Decimal => "decimal",
            Self::Search => "search",
        }
    }

    /// Parses a canonical HTML `inputmode` token (case-insensitive, trimmed) into [`InputMode`].
    pub fn from_whatwg_token(raw: &str) -> Option<Self> {
        let s = raw.trim();
        if s.eq_ignore_ascii_case("text") {
            Some(Self::Text)
        } else if s.eq_ignore_ascii_case("email") {
            Some(Self::Email)
        } else if s.eq_ignore_ascii_case("tel") {
            Some(Self::Tel)
        } else if s.eq_ignore_ascii_case("url") {
            Some(Self::Url)
        } else if s.eq_ignore_ascii_case("numeric") {
            Some(Self::Numeric)
        } else if s.eq_ignore_ascii_case("decimal") {
            Some(Self::Decimal)
        } else if s.eq_ignore_ascii_case("search") {
            Some(Self::Search)
        } else {
            None
        }
    }

    /// Canonical Apple `UIKeyboardType` identifier for iOS/iPadOS virtual keyboards.
    #[inline]
    pub const fn as_apple_keyboard_type(self) -> &'static str {
        match self {
            Self::Text => "default",
            Self::Email => "emailAddress",
            Self::Tel => "phonePad",
            Self::Url => "URL",
            Self::Numeric => "numberPad",
            Self::Decimal => "decimalPad",
            Self::Search => "webSearch",
        }
    }

    /// Canonical Android `android.text.InputType` symbolic flag combination.
    #[inline]
    pub const fn as_android_input_type(self) -> &'static str {
        match self {
            Self::Text => "TYPE_CLASS_TEXT",
            Self::Email => "TYPE_CLASS_TEXT|TYPE_TEXT_VARIATION_EMAIL_ADDRESS",
            Self::Tel => "TYPE_CLASS_PHONE",
            Self::Url => "TYPE_CLASS_TEXT|TYPE_TEXT_VARIATION_URI",
            Self::Numeric => "TYPE_CLASS_NUMBER",
            Self::Decimal => "TYPE_CLASS_NUMBER|TYPE_NUMBER_FLAG_DECIMAL",
            Self::Search => "TYPE_CLASS_TEXT|IME_ACTION_SEARCH",
        }
    }
}

impl fmt::Display for InputMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.whatwg_token())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autofill_hint_html_roundtrip_and_platform_mappings() {
        for &hint in AutofillHint::ALL {
            let html = hint.whatwg_token();
            assert_eq!(AutofillHint::from_whatwg_token(html), Some(hint));
            assert_eq!(
                AutofillHint::from_whatwg_token(&format!("  {}  ", html.to_ascii_uppercase())),
                Some(hint)
            );
            assert_eq!(hint.to_string(), html);
            if hint == AutofillHint::Off {
                assert_eq!(hint.as_apple_content_type(), None);
                assert_eq!(hint.as_android_autofill_hint(), None);
            } else {
                assert!(hint.as_apple_content_type().is_some());
                assert!(hint.as_android_autofill_hint().is_some());
            }
        }

        // Key credential & OTP platform tokens match Apple UIKit & Android Autofill specs.
        assert_eq!(AutofillHint::Username.as_apple_content_type(), Some("username"));
        assert_eq!(AutofillHint::Username.as_android_autofill_hint(), Some("username"));
        assert_eq!(AutofillHint::CurrentPassword.whatwg_token(), "current-password");
        assert_eq!(AutofillHint::CurrentPassword.as_apple_content_type(), Some("password"));
        assert_eq!(AutofillHint::CurrentPassword.as_android_autofill_hint(), Some("password"));
        assert_eq!(AutofillHint::NewPassword.as_apple_content_type(), Some("newPassword"));
        assert_eq!(AutofillHint::NewPassword.as_android_autofill_hint(), Some("newPassword"));
        assert_eq!(AutofillHint::OneTimeCode.whatwg_token(), "one-time-code");
        assert_eq!(AutofillHint::OneTimeCode.as_apple_content_type(), Some("oneTimeCode"));
        assert_eq!(AutofillHint::OneTimeCode.as_android_autofill_hint(), Some("smsOTPCode"));
        assert_eq!(AutofillHint::CcNumber.as_apple_content_type(), Some("creditCardNumber"));
        assert_eq!(AutofillHint::CcNumber.as_android_autofill_hint(), Some("creditCardNumber"));
        assert_eq!(AutofillHint::from_whatwg_token("unknown-token"), None);
    }

    #[test]
    fn input_mode_html_roundtrip_and_platform_mappings() {
        for &mode in InputMode::ALL {
            let html = mode.whatwg_token();
            assert_eq!(InputMode::from_whatwg_token(html), Some(mode));
            assert_eq!(
                InputMode::from_whatwg_token(&format!(" {} ", html.to_ascii_uppercase())),
                Some(mode)
            );
            assert_eq!(mode.to_string(), html);
            assert!(!mode.as_apple_keyboard_type().is_empty());
            assert!(!mode.as_android_input_type().is_empty());
        }
        assert_eq!(InputMode::Numeric.whatwg_token(), "numeric");
        assert_eq!(InputMode::Numeric.as_apple_keyboard_type(), "numberPad");
        assert_eq!(InputMode::Numeric.as_android_input_type(), "TYPE_CLASS_NUMBER");
        assert_eq!(InputMode::Decimal.as_apple_keyboard_type(), "decimalPad");
        assert_eq!(InputMode::from_whatwg_token("invalid"), None);
    }
}
