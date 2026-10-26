use std::{fmt, net::Ipv6Addr, str::FromStr};

use thiserror::Error;

pub const DEFAULT_MINECRAFT_PORT: u16 = 25565;

/// A Java Edition server address entered as `host`, `host:port`, `[ipv6]`, or
/// `[ipv6]:port`.
///
/// An address without an explicit port keeps `explicit_port = false` so launch
/// code can preserve the game's default-port and future SRV semantics.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ServerAddress {
    host: String,
    port: u16,
    explicit_port: bool,
}

impl ServerAddress {
    pub fn parse(input: &str) -> Result<Self, ServerAddressError> {
        let input = input.trim();
        if input.is_empty() {
            return Err(ServerAddressError::Empty);
        }
        if input.contains("://")
            || input.contains('/')
            || input.contains('?')
            || input.contains('#')
        {
            return Err(ServerAddressError::Unsupported);
        }
        if input.chars().any(|character| {
            character.is_whitespace() || character.is_control() || character == '@'
        }) {
            return Err(ServerAddressError::InvalidHost);
        }

        let (host, port, explicit_port) = if let Some(rest) = input.strip_prefix('[') {
            let Some(end) = rest.find(']') else {
                return Err(ServerAddressError::InvalidIpv6);
            };
            let host = &rest[..end];
            if host.parse::<Ipv6Addr>().is_err() {
                return Err(ServerAddressError::InvalidIpv6);
            }
            let suffix = &rest[end + 1..];
            if suffix.is_empty() {
                (host, DEFAULT_MINECRAFT_PORT, false)
            } else if let Some(port) = suffix.strip_prefix(':') {
                (host, parse_port(port)?, true)
            } else {
                return Err(ServerAddressError::InvalidHost);
            }
        } else if input.matches(':').count() > 1 {
            if input.parse::<Ipv6Addr>().is_err() {
                return Err(ServerAddressError::InvalidIpv6);
            }
            (input, DEFAULT_MINECRAFT_PORT, false)
        } else if let Some((host, port)) = input.rsplit_once(':') {
            (host, parse_port(port)?, true)
        } else {
            (input, DEFAULT_MINECRAFT_PORT, false)
        };

        if host.is_empty() {
            return Err(ServerAddressError::EmptyHost);
        }
        Ok(Self {
            host: host.to_owned(),
            port,
            explicit_port,
        })
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn has_explicit_port(&self) -> bool {
        self.explicit_port
    }

    /// Address passed to `--quickPlayMultiplayer`.
    ///
    /// A missing explicit port is deliberately omitted so the client can apply
    /// its normal default/SRV behavior.
    pub fn quick_play_address(&self) -> String {
        if self.explicit_port {
            self.to_string()
        } else if self.host.parse::<Ipv6Addr>().is_ok() {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        }
    }

    /// Host passed to the legacy `--server` argument, without IPv6 brackets.
    pub fn legacy_host(&self) -> &str {
        &self.host
    }
}

impl FromStr for ServerAddress {
    type Err = ServerAddressError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        Self::parse(input)
    }
}

impl fmt::Display for ServerAddress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.host.parse::<Ipv6Addr>().is_ok() {
            if self.explicit_port {
                write!(formatter, "[{}]:{}", self.host, self.port)
            } else {
                write!(formatter, "[{}]", self.host)
            }
        } else if self.explicit_port {
            write!(formatter, "{}:{}", self.host, self.port)
        } else {
            formatter.write_str(&self.host)
        }
    }
}

fn parse_port(value: &str) -> Result<u16, ServerAddressError> {
    let port = value
        .parse::<u16>()
        .map_err(|_| ServerAddressError::InvalidPort(value.to_owned()))?;
    if port == 0 {
        return Err(ServerAddressError::InvalidPort(value.to_owned()));
    }
    Ok(port)
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ServerAddressError {
    #[error("server address is empty")]
    Empty,
    #[error("server address must not contain a scheme, path, query, or fragment")]
    Unsupported,
    #[error("server host is empty")]
    EmptyHost,
    #[error("server host contains whitespace, control characters, or `@`")]
    InvalidHost,
    #[error("invalid IPv6 server address")]
    InvalidIpv6,
    #[error("invalid server port `{0}`")]
    InvalidPort(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hosts_and_ports() {
        let address = ServerAddress::parse("play.example.com").unwrap();
        assert_eq!(address.host(), "play.example.com");
        assert_eq!(address.port(), DEFAULT_MINECRAFT_PORT);
        assert!(!address.has_explicit_port());
        assert_eq!(address.to_string(), "play.example.com");
        assert_eq!(address.quick_play_address(), "play.example.com");

        let address = ServerAddress::parse("play.example.com:25566").unwrap();
        assert_eq!(address.host(), "play.example.com");
        assert_eq!(address.port(), 25566);
        assert!(address.has_explicit_port());
        assert_eq!(address.to_string(), "play.example.com:25566");
        assert_eq!(address.quick_play_address(), "play.example.com:25566");
    }

    #[test]
    fn parses_ipv6_with_and_without_ports() {
        let address = ServerAddress::parse("[::1]").unwrap();
        assert_eq!(address.host(), "::1");
        assert_eq!(address.port(), DEFAULT_MINECRAFT_PORT);
        assert_eq!(address.to_string(), "[::1]");
        assert_eq!(address.quick_play_address(), "[::1]");
        assert_eq!(address.legacy_host(), "::1");

        let address = ServerAddress::parse("[::1]:25566").unwrap();
        assert_eq!(address.port(), 25566);
        assert_eq!(address.to_string(), "[::1]:25566");
        assert_eq!(address.quick_play_address(), "[::1]:25566");

        let address = ServerAddress::parse("::1").unwrap();
        assert_eq!(address.host(), "::1");
        assert_eq!(address.to_string(), "[::1]");
    }

    #[test]
    fn rejects_invalid_addresses() {
        for input in [
            "",
            "   ",
            "https://example.com",
            "example.com/path",
            "example.com:",
            "example.com:0",
            "example.com:65536",
            ":25565",
            "[::1",
            "[not-ipv6]:25565",
            "a:b:c",
            "user@example.com",
            "example.com:port",
        ] {
            assert!(
                ServerAddress::parse(input).is_err(),
                "expected `{input}` to be rejected"
            );
        }
    }
}
