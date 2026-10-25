use std::{fmt, str::FromStr};

use thiserror::Error;

/// A validated 160-bit SHA-1 digest from Minecraft metadata.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Sha1Hash([u8; 20]);

impl Sha1Hash {
    pub const fn from_bytes(bytes: [u8; 20]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 20] {
        &self.0
    }
}

impl FromStr for Sha1Hash {
    type Err = ParseSha1Error;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let mut bytes = [0_u8; 20];
        hex::decode_to_slice(input, &mut bytes).map_err(|source| ParseSha1Error {
            input: input.to_owned(),
            source,
        })?;
        Ok(Self(bytes))
    }
}

impl fmt::Display for Sha1Hash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&hex::encode(self.0))
    }
}

impl fmt::Debug for Sha1Hash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

#[derive(Debug, Error)]
#[error("invalid SHA-1 `{input}`: {source}")]
pub struct ParseSha1Error {
    input: String,
    #[source]
    source: hex::FromHexError,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_formats_sha1() {
        let hash: Sha1Hash = "0123456789abcdef0123456789abcdef01234567".parse().unwrap();
        assert_eq!(hash.to_string(), "0123456789abcdef0123456789abcdef01234567");
        assert_eq!(hash.as_bytes()[0], 0x01);
    }

    #[test]
    fn constructs_from_raw_digest() {
        let hash = Sha1Hash::from_bytes([0xab; 20]);
        assert_eq!(hash.as_bytes(), &[0xab; 20]);
    }

    #[test]
    fn normalizes_uppercase_hex() {
        let hash: Sha1Hash = "ABCDEFABCDEFABCDEFABCDEFABCDEFABCDEFABCD".parse().unwrap();
        assert_eq!(hash.to_string(), "abcdefabcdefabcdefabcdefabcdefabcdefabcd");
    }

    #[test]
    fn rejects_invalid_length() {
        assert!("abcd".parse::<Sha1Hash>().is_err());
    }

    #[test]
    fn rejects_non_hex_input() {
        assert!(
            "zz23456789abcdef0123456789abcdef01234567"
                .parse::<Sha1Hash>()
                .is_err()
        );
    }
}
