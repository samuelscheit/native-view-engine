//! Zero-dependency `define_bitflags!` macro for `#[repr(transparent)]` typed bitmask newtypes.
//!
//! Eliminates repetitive hand-written bitmask boilerplate (`NONE`, `ALL`, `bits`,
//! `from_bits_truncate`, `contains`, `intersects`, `union`, `difference`, `insert`, `remove`,
//! `set`, `BitOr`/`BitAnd`/`BitXor`/`Not`, and flag-name `Debug` formatting) across the
//! workspace with zero external crates and zero proc-macro compile overhead.

/// Defines a `#[repr(transparent)]` bitmask newtype backed by an unsigned integer primitive
/// (`u8`, `u16`, `u32`, `u64`, `usize`).
///
/// Additional derives (e.g., `Default`, `Serialize`, `Deserialize`) can be passed in attributes
/// above the struct; `Clone`, `Copy`, `PartialEq`, `Eq`, `Hash`, and named-flag `Debug` are
/// implemented automatically.
#[macro_export]
macro_rules! define_bitflags {
    (
        $(#[$meta:meta])*
        $vis:vis struct $name:ident($repr:ty) {
            $(
                $(#[$flag_meta:meta])*
                const $flag:ident = $val:expr;
            )*
        }
    ) => {
        $(#[$meta])*
        #[repr(transparent)]
        #[derive(Clone, Copy, PartialEq, Eq, Hash)]
        $vis struct $name($repr);

        const _: () = {
            assert!(core::mem::size_of::<$name>() == core::mem::size_of::<$repr>());
            assert!(core::mem::align_of::<$name>() == core::mem::align_of::<$repr>());
        };

        #[allow(dead_code)]
        impl $name {
            /// Empty bitmask with no flags set.
            pub const NONE: Self = Self(0);
            /// Alias for [`Self::NONE`].
            pub const EMPTY: Self = Self(0);
            $(
                $(#[$flag_meta])*
                pub const $flag: Self = Self($val);
            )*
            /// Bitwise union of all declared flags.
            pub const ALL: Self = Self(0 $(| $val)*);

            /// Returns an empty bitmask (const helper).
            #[inline]
            pub const fn empty() -> Self {
                Self(0)
            }

            /// Returns the underlying integer bitmask.
            #[inline]
            pub const fn bits(self) -> $repr {
                self.0
            }

            /// Constructs from raw bits, masking off any undeclared bits.
            #[inline]
            pub const fn from_bits_truncate(bits: $repr) -> Self {
                Self(bits & Self::ALL.0)
            }

            /// Constructs from raw bits without masking.
            #[inline]
            pub const fn from_bits_retain(bits: $repr) -> Self {
                Self(bits)
            }

            /// Returns `true` when no flags are set.
            #[inline]
            pub const fn is_empty(self) -> bool {
                self.0 == 0
            }

            /// Returns `true` when every flag in `other` is set in `self`.
            #[inline]
            pub const fn contains(self, other: Self) -> bool {
                (self.0 & other.0) == other.0
            }

            /// Returns `true` when at least one flag in `other` is set in `self`.
            #[inline]
            pub const fn intersects(self, other: Self) -> bool {
                (self.0 & other.0) != 0
            }

            /// Bitwise OR (`self | other`), usable in `const` contexts.
            #[inline]
            pub const fn union(self, other: Self) -> Self {
                Self(self.0 | other.0)
            }

            /// Bitwise AND (`self & other`), usable in `const` contexts.
            #[inline]
            pub const fn intersection(self, other: Self) -> Self {
                Self(self.0 & other.0)
            }

            /// Bitwise difference (`self & !other`), usable in `const` contexts.
            #[inline]
            pub const fn difference(self, other: Self) -> Self {
                Self(self.0 & !other.0)
            }

            /// Returns `self` with `other` flags added (const alias for [`Self::union`]).
            #[inline]
            pub const fn with(self, other: Self) -> Self {
                Self(self.0 | other.0)
            }

            /// Returns `self` with `other` flags cleared (const alias for [`Self::difference`]).
            #[inline]
            pub const fn without(self, other: Self) -> Self {
                Self(self.0 & !other.0)
            }

            /// Sets all flags in `other`.
            #[inline]
            pub fn insert(&mut self, other: Self) {
                self.0 |= other.0;
            }

            /// Clears all flags in `other`.
            #[inline]
            pub fn remove(&mut self, other: Self) {
                self.0 &= !other.0;
            }

            /// Sets or clears `other` depending on `on`.
            #[inline]
            pub fn set(&mut self, other: Self, on: bool) {
                if on {
                    self.0 |= other.0;
                } else {
                    self.0 &= !other.0;
                }
            }
        }

        impl core::fmt::Debug for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str(concat!(stringify!($name), "("))?;
                let mut first = true;
                $(
                    if ($val != 0) && ((self.0 & $val) == $val) {
                        if !first {
                            f.write_str(" | ")?;
                        }
                        f.write_str(stringify!($flag))?;
                        first = false;
                    }
                )*
                if first {
                    f.write_str("NONE")?;
                }
                f.write_str(")")
            }
        }

        impl core::ops::BitOr for $name {
            type Output = Self;
            #[inline]
            fn bitor(self, rhs: Self) -> Self {
                Self(self.0 | rhs.0)
            }
        }

        impl core::ops::BitOrAssign for $name {
            #[inline]
            fn bitor_assign(&mut self, rhs: Self) {
                self.0 |= rhs.0;
            }
        }

        impl core::ops::BitAnd for $name {
            type Output = Self;
            #[inline]
            fn bitand(self, rhs: Self) -> Self {
                Self(self.0 & rhs.0)
            }
        }

        impl core::ops::BitAndAssign for $name {
            #[inline]
            fn bitand_assign(&mut self, rhs: Self) {
                self.0 &= rhs.0;
            }
        }

        impl core::ops::BitXor for $name {
            type Output = Self;
            #[inline]
            fn bitxor(self, rhs: Self) -> Self {
                Self(self.0 ^ rhs.0)
            }
        }

        impl core::ops::BitXorAssign for $name {
            #[inline]
            fn bitxor_assign(&mut self, rhs: Self) {
                self.0 ^= rhs.0;
            }
        }

        impl core::ops::Not for $name {
            type Output = Self;
            #[inline]
            fn not(self) -> Self {
                Self(!self.0 & Self::ALL.0)
            }
        }
    };
}

#[cfg(test)]
mod tests {
    define_bitflags! {
        #[derive(Default)]
        struct SampleMask(u8) {
            const ALPHA = 1 << 0;
            const BETA = 1 << 1;
            const GAMMA = 1 << 2;
        }
    }

    #[test]
    fn bitflags_ops_truncate_and_debug_work() {
        assert_eq!(SampleMask::default(), SampleMask::NONE);
        assert!(SampleMask::NONE.is_empty());
        assert_eq!(SampleMask::ALL.bits(), 0b111);
        assert_eq!(SampleMask::from_bits_truncate(0xFF), SampleMask::ALL);

        let mut m = SampleMask::ALPHA | SampleMask::GAMMA;
        assert!(m.contains(SampleMask::ALPHA));
        assert!(!m.contains(SampleMask::BETA));
        assert!(m.intersects(SampleMask::BETA | SampleMask::GAMMA));
        assert_eq!(format!("{m:?}"), "SampleMask(ALPHA | GAMMA)");
        assert_eq!(format!("{:?}", SampleMask::NONE), "SampleMask(NONE)");

        m.set(SampleMask::BETA, true);
        assert_eq!(m, SampleMask::ALL);
        m.remove(SampleMask::ALPHA);
        assert_eq!(!m, SampleMask::ALPHA);
        assert_eq!(m.without(SampleMask::BETA), SampleMask::GAMMA);
    }
}
