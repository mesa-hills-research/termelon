//! Sets of named flags in an integer, for `Modifier` and `KeyModifiers`.

/// Defines a newtype over an unsigned integer with named flags, set operations as `const fn`s, the
/// `|`, `&`, `-` and `!` operators with their assigning forms, and a `Debug` that names the flags.
macro_rules! flags {
    (
        $(#[$meta:meta])*
        pub struct $name:ident($bits:ty);
        empty = $empty:literal;
        $(
            $(#[$flag_meta:meta])*
            const $flag:ident = $value:expr;
        )+
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
        pub struct $name($bits);

        impl $name {
            $(
                $(#[$flag_meta])*
                pub const $flag: Self = Self($value);
            )+

            const NAMES: &'static [(&'static str, Self)] = &[$((stringify!($flag), Self::$flag)),+];

            /// No flags.
            pub const fn empty() -> Self {
                Self(0)
            }

            /// Every flag.
            pub const fn all() -> Self {
                Self(0 $(| $value)+)
            }

            /// The flags as bits.
            pub const fn bits(self) -> $bits {
                self.0
            }

            /// The flags among `bits`, without the bits that name no flag.
            pub const fn from_bits_truncate(bits: $bits) -> Self {
                Self(bits & Self::all().0)
            }

            /// Whether no flag is set.
            pub const fn is_empty(self) -> bool {
                self.0 == 0
            }

            /// Whether every flag of `other` is set.
            pub const fn contains(self, other: Self) -> bool {
                self.0 & other.0 == other.0
            }

            /// Whether any flag of `other` is set.
            pub const fn intersects(self, other: Self) -> bool {
                self.0 & other.0 != 0
            }

            /// Sets the flags of `other`.
            pub const fn insert(&mut self, other: Self) {
                self.0 |= other.0;
            }

            /// Clears the flags of `other`.
            pub const fn remove(&mut self, other: Self) {
                self.0 &= !other.0;
            }

            /// The flags set in either, as `|` gives.
            #[must_use]
            pub const fn union(self, other: Self) -> Self {
                Self(self.0 | other.0)
            }

            /// The flags set in both, as `&` gives.
            #[must_use]
            pub const fn intersection(self, other: Self) -> Self {
                Self(self.0 & other.0)
            }

            /// The flags of `self` that `other` doesn't have, as `-` gives.
            #[must_use]
            pub const fn difference(self, other: Self) -> Self {
                Self(self.0 & !other.0)
            }

            /// The flags that aren't set, as `!` gives.
            #[must_use]
            pub const fn complement(self) -> Self {
                Self(!self.0 & Self::all().0)
            }
        }

        impl core::ops::BitOr for $name {
            type Output = Self;
            fn bitor(self, other: Self) -> Self {
                self.union(other)
            }
        }

        impl core::ops::BitOrAssign for $name {
            fn bitor_assign(&mut self, other: Self) {
                self.insert(other);
            }
        }

        impl core::ops::BitAnd for $name {
            type Output = Self;
            fn bitand(self, other: Self) -> Self {
                self.intersection(other)
            }
        }

        impl core::ops::BitAndAssign for $name {
            fn bitand_assign(&mut self, other: Self) {
                *self = self.intersection(other);
            }
        }

        impl core::ops::Sub for $name {
            type Output = Self;
            fn sub(self, other: Self) -> Self {
                self.difference(other)
            }
        }

        impl core::ops::SubAssign for $name {
            fn sub_assign(&mut self, other: Self) {
                self.remove(other);
            }
        }

        impl core::ops::Not for $name {
            type Output = Self;
            fn not(self) -> Self {
                self.complement()
            }
        }

        impl core::fmt::Debug for $name {
            /// Names the flags, as in `Modifier(BOLD | ITALIC)`.
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str(concat!(stringify!($name), "("))?;
                if self.is_empty() {
                    f.write_str($empty)?;
                }
                let mut first = true;
                for &(name, flag) in Self::NAMES {
                    if self.contains(flag) {
                        if !first {
                            f.write_str(" | ")?;
                        }
                        f.write_str(name)?;
                        first = false;
                    }
                }
                f.write_str(")")
            }
        }
    };
}

pub(crate) use flags;
