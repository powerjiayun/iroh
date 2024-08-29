use serde::{de, Deserializer, Serializer};
use willow_encoding::sync::{Decodable, Encodable};

pub fn serialize<T: Encodable, S: Serializer>(
    encodable: &T,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let vec = super::codec2::to_vec(encodable);
    serde_bytes::serialize(&vec, serializer)
}

pub fn deserialize<'de, D: Deserializer<'de>, T: Decodable>(
    deserializer: D,
) -> Result<T, D::Error> {
    let bytes: Vec<u8> = serde_bytes::deserialize(deserializer)?;
    let decoded = super::codec2::from_bytes(&bytes).map_err(de::Error::custom)?;
    Ok(decoded)
}
