//! `raw_enum!`: a Swift `enum Name: String` as a Rust enum with `as_str`, `from_raw_value`, `ALL` and serde support
//! that reads and writes the raw values (conventions section 4).

macro_rules! raw_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident {
            $($(#[$vmeta:meta])* $variant:ident = $raw:literal),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        $vis enum $name {
            $($(#[$vmeta])* $variant),+
        }

        impl $name {
            /// Every case, in declaration order (Swift `CaseIterable`).
            pub const ALL: &'static [$name] = &[$($name::$variant),+];

            /// The Swift raw value.
            pub fn as_str(&self) -> &'static str {
                match self {
                    $($name::$variant => $raw),+
                }
            }

            /// `init?(rawValue:)`: `None` for text that is not a raw value (case matters).
            pub fn from_raw_value(raw: &str) -> Option<$name> {
                match raw {
                    $($raw => Some($name::$variant),)+
                    _ => None,
                }
            }
        }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<$name, D::Error> {
                let raw = <std::borrow::Cow<'de, str> as serde::Deserialize>::deserialize(deserializer)?;
                $name::from_raw_value(&raw).ok_or_else(|| {
                    <D::Error as serde::de::Error>::custom(format!(
                        "unknown {} value {raw:?}",
                        stringify!($name)
                    ))
                })
            }
        }
    };
}

pub(crate) use raw_enum;
