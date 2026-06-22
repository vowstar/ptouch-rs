// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Serde helper that stores a byte buffer as a base64 string.
//!
//! Apply with `#[serde(with = "base64_bytes")]` on a `Vec<u8>` field so the
//! serialized form is a compact, human-readable string instead of an array of
//! integers. This keeps embedded image data on a single readable line.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Deserializer, Serializer};

/// Serialize a byte slice as a base64 string.
pub fn serialize<S>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&STANDARD.encode(bytes))
}

/// Deserialize a base64 string back into a byte buffer.
pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
where
    D: Deserializer<'de>,
{
    let encoded = String::deserialize(deserializer)?;
    STANDARD
        .decode(encoded.as_bytes())
        .map_err(serde::de::Error::custom)
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Holder {
        #[serde(with = "super")]
        data: Vec<u8>,
    }

    #[test]
    fn test_round_trip_through_toml() {
        let original = Holder {
            data: vec![0, 1, 2, 250, 251, 255],
        };
        let text = toml::to_string(&original).unwrap();
        // The bytes must be stored as a quoted string, not an integer array.
        assert!(text.contains('"'));
        let parsed: Holder = toml::from_str(&text).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn test_empty_round_trip() {
        let original = Holder { data: Vec::new() };
        let text = toml::to_string(&original).unwrap();
        let parsed: Holder = toml::from_str(&text).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn test_invalid_base64_errors() {
        // '!' is not part of the base64 alphabet.
        let text = "data = \"not valid base64 !!!\"\n";
        let result: std::result::Result<Holder, _> = toml::from_str(text);
        assert!(result.is_err());
    }
}
