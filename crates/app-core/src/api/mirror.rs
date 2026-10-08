//! API enums that mirror an enum of the `storage` crate.
//!
//! `storage` does not know about TypeScript, and the browser must not see a
//! hand-written copy of its vocabularies. The macro defines the API enum with the
//! same wire text and writes both conversions as exhaustive matches, so a
//! variant added to the storage enum stops the build here until the API enum
//! follows.

macro_rules! api_enum {
    (
        $(#[$meta:meta])*
        $name:ident <=> $storage:ty {
            $( $(#[$vmeta:meta])* $variant:ident => $text:literal ),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, ts_rs::TS,
        )]
        #[ts(export)]
        pub enum $name {
            $( $(#[$vmeta])* #[serde(rename = $text)] $variant ),+
        }

        impl From<$storage> for $name {
            fn from(value: $storage) -> Self {
                match value { $( <$storage>::$variant => Self::$variant ),+ }
            }
        }

        impl From<$name> for $storage {
            fn from(value: $name) -> Self {
                match value { $( $name::$variant => <$storage>::$variant ),+ }
            }
        }
    };
}

pub(crate) use api_enum;
