//! Byte strings in JSON arguments and views, written as lowercase hex.
//! Input may be upper or lower case, with no `0x` prefix.

use near_sdk::serde::{de, Deserialize, Deserializer, Serialize, Serializer};

/// Exactly 32 bytes as 64 hex characters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hex32(pub [u8; 32]);

/// Exactly 64 bytes as 128 hex characters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hex64(pub [u8; 64]);

/// Any number of bytes as hex.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HexBytes(pub Vec<u8>);

fn decode_fixed<'de, D: Deserializer<'de>, const N: usize>(d: D) -> Result<[u8; N], D::Error> {
    let s = String::deserialize(d)?;
    let mut out = [0u8; N];
    hex::decode_to_slice(&s, &mut out)
        .map_err(|e| de::Error::custom(format!("expected {} hex characters: {e}", 2 * N)))?;
    Ok(out)
}

impl Serialize for Hex32 {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex::encode(self.0))
    }
}

impl<'de> Deserialize<'de> for Hex32 {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        decode_fixed::<D, 32>(d).map(Hex32)
    }
}

impl Serialize for Hex64 {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex::encode(self.0))
    }
}

impl<'de> Deserialize<'de> for Hex64 {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        decode_fixed::<D, 64>(d).map(Hex64)
    }
}

impl Serialize for HexBytes {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex::encode(&self.0))
    }
}

impl<'de> Deserialize<'de> for HexBytes {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        hex::decode(&s).map(HexBytes).map_err(de::Error::custom)
    }
}
