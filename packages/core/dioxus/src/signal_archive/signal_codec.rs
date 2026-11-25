//! Typed `SignalCodec` trait and primitive implementations for `nve-dioxus::signal_archive`.

use super::SignalValue;

/// Typed conversion between Rust signal types and direct binary FFI [`SignalValue`].
pub trait SignalCodec: Sized {
    fn to_signal_value(&self) -> SignalValue;
    fn from_signal_value(value: &SignalValue) -> Option<Self>;
}

/// Wrapper type for storing validated `NveBinValue` bytes inside a [`super::SignalRegistry`] slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinValueSlot(pub Vec<u8>);

impl SignalCodec for SignalValue {
    fn to_signal_value(&self) -> SignalValue {
        self.clone()
    }

    fn from_signal_value(value: &SignalValue) -> Option<Self> {
        Some(value.clone())
    }
}

impl SignalCodec for bool {
    fn to_signal_value(&self) -> SignalValue {
        SignalValue::Bool(*self)
    }

    fn from_signal_value(value: &SignalValue) -> Option<Self> {
        match value {
            SignalValue::Bool(v) => Some(*v),
            _ => None,
        }
    }
}

impl SignalCodec for i64 {
    fn to_signal_value(&self) -> SignalValue {
        SignalValue::I64(*self)
    }

    fn from_signal_value(value: &SignalValue) -> Option<Self> {
        match value {
            SignalValue::I64(v) => Some(*v),
            _ => None,
        }
    }
}

impl SignalCodec for i32 {
    fn to_signal_value(&self) -> SignalValue {
        SignalValue::I64(i64::from(*self))
    }

    fn from_signal_value(value: &SignalValue) -> Option<Self> {
        match value {
            SignalValue::I64(v) => i32::try_from(*v).ok(),
            _ => None,
        }
    }
}

impl SignalCodec for i16 {
    fn to_signal_value(&self) -> SignalValue {
        SignalValue::I64(i64::from(*self))
    }

    fn from_signal_value(value: &SignalValue) -> Option<Self> {
        match value {
            SignalValue::I64(v) => i16::try_from(*v).ok(),
            _ => None,
        }
    }
}

impl SignalCodec for i8 {
    fn to_signal_value(&self) -> SignalValue {
        SignalValue::I64(i64::from(*self))
    }

    fn from_signal_value(value: &SignalValue) -> Option<Self> {
        match value {
            SignalValue::I64(v) => i8::try_from(*v).ok(),
            _ => None,
        }
    }
}

impl SignalCodec for isize {
    fn to_signal_value(&self) -> SignalValue {
        SignalValue::I64(*self as i64)
    }

    fn from_signal_value(value: &SignalValue) -> Option<Self> {
        match value {
            SignalValue::I64(v) => isize::try_from(*v).ok(),
            _ => None,
        }
    }
}

impl SignalCodec for u64 {
    fn to_signal_value(&self) -> SignalValue {
        SignalValue::U64(*self)
    }

    fn from_signal_value(value: &SignalValue) -> Option<Self> {
        match value {
            SignalValue::U64(v) => Some(*v),
            _ => None,
        }
    }
}

impl SignalCodec for u32 {
    fn to_signal_value(&self) -> SignalValue {
        SignalValue::U64(u64::from(*self))
    }

    fn from_signal_value(value: &SignalValue) -> Option<Self> {
        match value {
            SignalValue::U64(v) => u32::try_from(*v).ok(),
            _ => None,
        }
    }
}

impl SignalCodec for u16 {
    fn to_signal_value(&self) -> SignalValue {
        SignalValue::U64(u64::from(*self))
    }

    fn from_signal_value(value: &SignalValue) -> Option<Self> {
        match value {
            SignalValue::U64(v) => u16::try_from(*v).ok(),
            _ => None,
        }
    }
}

impl SignalCodec for u8 {
    fn to_signal_value(&self) -> SignalValue {
        SignalValue::U64(u64::from(*self))
    }

    fn from_signal_value(value: &SignalValue) -> Option<Self> {
        match value {
            SignalValue::U64(v) => u8::try_from(*v).ok(),
            _ => None,
        }
    }
}

impl SignalCodec for usize {
    fn to_signal_value(&self) -> SignalValue {
        SignalValue::U64(*self as u64)
    }

    fn from_signal_value(value: &SignalValue) -> Option<Self> {
        match value {
            SignalValue::U64(v) => usize::try_from(*v).ok(),
            _ => None,
        }
    }
}

impl SignalCodec for f64 {
    fn to_signal_value(&self) -> SignalValue {
        SignalValue::F64(*self)
    }

    fn from_signal_value(value: &SignalValue) -> Option<Self> {
        match value {
            SignalValue::F64(v) => Some(*v),
            _ => None,
        }
    }
}

impl SignalCodec for f32 {
    fn to_signal_value(&self) -> SignalValue {
        SignalValue::F64(f64::from(*self))
    }

    fn from_signal_value(value: &SignalValue) -> Option<Self> {
        match value {
            SignalValue::F64(v) => Some(*v as f32),
            _ => None,
        }
    }
}

impl SignalCodec for String {
    fn to_signal_value(&self) -> SignalValue {
        SignalValue::Str(self.clone())
    }

    fn from_signal_value(value: &SignalValue) -> Option<Self> {
        match value {
            SignalValue::Str(s) => Some(s.clone()),
            _ => None,
        }
    }
}

impl SignalCodec for Vec<u8> {
    fn to_signal_value(&self) -> SignalValue {
        SignalValue::Bytes(self.clone())
    }

    fn from_signal_value(value: &SignalValue) -> Option<Self> {
        match value {
            SignalValue::Bytes(b) => Some(b.clone()),
            _ => None,
        }
    }
}

impl SignalCodec for BinValueSlot {
    fn to_signal_value(&self) -> SignalValue {
        SignalValue::BinValue(self.0.clone())
    }

    fn from_signal_value(value: &SignalValue) -> Option<Self> {
        match value {
            SignalValue::BinValue(b) => Some(Self(b.clone())),
            _ => None,
        }
    }
}

impl<T: SignalCodec> SignalCodec for Option<T> {
    fn to_signal_value(&self) -> SignalValue {
        match self {
            Some(v) => v.to_signal_value(),
            None => SignalValue::Null,
        }
    }

    fn from_signal_value(value: &SignalValue) -> Option<Self> {
        match value {
            SignalValue::Null => Some(None),
            other => T::from_signal_value(other).map(Some),
        }
    }
}
