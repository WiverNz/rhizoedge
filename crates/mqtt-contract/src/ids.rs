//! Validated protocol identifiers.

use alloc::string::{String, ToString};
use core::{fmt, str::FromStr};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use uuid::Uuid;

/// A device-scoped identifier with the MQTT-safe grammar from ADR-012.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DeviceId(String);

/// Why a [`DeviceId`] was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeviceIdError {
    TooShort,
    TooLong,
    InvalidCharacter,
    InvalidBoundary,
}

impl DeviceId {
    /// Validates and constructs an identifier. This is the only constructor.
    pub fn parse(value: &str) -> Result<Self, DeviceIdError> {
        let len = value.len();
        if len < 3 {
            return Err(DeviceIdError::TooShort);
        }
        if len > 32 {
            return Err(DeviceIdError::TooLong);
        }
        if !value
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        {
            return Err(DeviceIdError::InvalidCharacter);
        }
        if value.starts_with('-') || value.ends_with('-') {
            return Err(DeviceIdError::InvalidBoundary);
        }
        Ok(Self(value.to_string()))
    }
}
impl AsRef<str> for DeviceId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}
impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl FromStr for DeviceId {
    type Err = DeviceIdError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}
impl Serialize for DeviceId {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}
impl<'de> Deserialize<'de> for DeviceId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Self::parse(&s).map_err(|_| de::Error::custom("invalid device id"))
    }
}
impl fmt::Display for DeviceIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid device id: {self:?}")
    }
}
#[cfg(feature = "std")]
impl std::error::Error for DeviceIdError {}

/// Minimal randomness source accepted by UUID generators without tying the
/// contract to an operating-system RNG.
pub trait RandomSource {
    /// Fills every output byte with caller-provided randomness.
    fn fill_bytes(&mut self, output: &mut [u8]);
}

macro_rules! uuid_id {
    ($name:ident) => {
        #[doc = concat!(stringify!($name), " UUID identifier.")]
        #[derive(
            Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(Uuid);
        impl $name {
            /// Wraps an already generated UUID.
            pub const fn from_uuid(value: Uuid) -> Self {
                Self(value)
            }
            /// Returns the underlying UUID.
            pub const fn as_uuid(self) -> Uuid {
                self.0
            }
            /// Generates a UUIDv7 from caller-provided time and randomness.
            pub fn new_v7(now: crate::UtcMillis, rng: &mut impl RandomSource) -> Self {
                let mut bytes = [0u8; 16];
                rng.fill_bytes(&mut bytes);
                let timestamp = (now.0 as u64) & 0x0000_ffff_ffff_ffff;
                bytes[..6].copy_from_slice(&timestamp.to_be_bytes()[2..]);
                bytes[6] = (bytes[6] & 0x0f) | 0x70;
                bytes[8] = (bytes[8] & 0x3f) | 0x80;
                Self(Uuid::from_bytes(bytes))
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}
uuid_id!(CommandId);
uuid_id!(MessageId);
uuid_id!(BootId);
uuid_id!(EventId);

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;
    use proptest::prelude::*;
    #[test]
    fn boundaries_and_adversarial_values() {
        for valid in ["abc", "plant-node-01", "a123456789012345678901234567890b"] {
            assert!(DeviceId::parse(valid).is_ok());
        }
        for invalid in [
            "x/#",
            "+",
            "#",
            "Plant-01",
            "ab",
            "a123456789012345678901234567890bc",
            "-abc",
            "abc-",
            "plant node",
            "a/b",
        ] {
            assert!(DeviceId::parse(invalid).is_err(), "{invalid}");
        }
    }
    #[test]
    fn serde_validates() {
        assert!(serde_json::from_str::<DeviceId>("\"x/#\"").is_err());
        let id = DeviceId::parse("node-01").unwrap();
        assert_eq!(
            serde_json::from_str::<DeviceId>(&serde_json::to_string(&id).unwrap()).unwrap(),
            id
        );
    }
    struct Fixed(u8);
    impl RandomSource for Fixed {
        fn fill_bytes(&mut self, out: &mut [u8]) {
            out.fill(self.0);
            self.0 = self.0.wrapping_add(1);
        }
    }
    #[test]
    fn uuid_v7_orders_by_caller_time() {
        let mut rng = Fixed(1);
        let early = MessageId::new_v7(crate::UtcMillis(1000), &mut rng);
        let late = MessageId::new_v7(crate::UtcMillis(1001), &mut rng);
        assert!(early < late);
        assert_eq!(early.as_uuid().get_version_num(), 7);
    }
    proptest! { #[test] fn forbidden_characters_fail(s in "[a-z0-9-]{0,20}", bad in prop_oneof![Just('/'),Just('+'),Just('#'),Just('A')]) { let candidate=format!("a{s}{bad}z"); prop_assert!(DeviceId::parse(&candidate).is_err()); } }
}
