use std::{collections::HashSet, sync::OnceLock};

use rmcl_domain::{ServerAddress, ServerAddressError};
use serde::Deserialize;
use thiserror::Error;
use url::Url;

const FEATURED_SERVERS_JSON: &str = include_str!("../assets/featured_servers.json");

/// A curated server shown in the server discovery tab.
///
/// The list is shipped with the application. Status, MOTD, player counts and
/// icons are queried separately with Server List Ping and never persisted in
/// this model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeaturedServer {
    pub id: String,
    pub name: String,
    pub address: ServerAddress,
    pub version: Option<String>,
    pub version_note: Option<String>,
    pub description: String,
    pub tags: Vec<String>,
    pub requires_mods: bool,
    pub source_url: Option<String>,
}

impl FeaturedServer {
    /// Whether the discovery entry is a Vanilla server that can be joined
    /// directly with the user's selected instance.
    pub fn can_join(&self) -> bool {
        !self.requires_mods
    }
}

/// Return the curated list shipped with this build.
///
/// The embedded JSON is validated by tests. If it is ever corrupted, the UI
/// receives an error string instead of a panic.
pub fn featured_servers() -> Result<&'static [FeaturedServer], &'static str> {
    static SERVERS: OnceLock<Result<Vec<FeaturedServer>, String>> = OnceLock::new();
    match SERVERS.get_or_init(|| {
        parse_featured_servers(FEATURED_SERVERS_JSON).map_err(|error| error.to_string())
    }) {
        Ok(servers) => Ok(servers),
        Err(error) => Err(error),
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FeaturedServerSpec {
    id: String,
    name: String,
    address: String,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    version_note: Option<String>,
    description: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    requires_mods: bool,
    #[serde(default)]
    source_url: Option<String>,
}

#[derive(Debug, Error)]
pub enum FeaturedServerError {
    #[error("featured server data is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("featured server `{id}` has an invalid address `{address}`: {source}")]
    Address {
        id: String,
        address: String,
        #[source]
        source: ServerAddressError,
    },
    #[error("featured server `{id}` has an invalid source URL `{url}`: {source}")]
    Url {
        id: String,
        url: String,
        #[source]
        source: url::ParseError,
    },
    #[error("featured server `{id}` is invalid: {reason}")]
    Invalid { id: String, reason: String },
    #[error("featured server id `{0}` appears more than once")]
    DuplicateId(String),
}

fn parse_featured_servers(json: &str) -> Result<Vec<FeaturedServer>, FeaturedServerError> {
    let specs: Vec<FeaturedServerSpec> = serde_json::from_str(json)?;
    let mut ids = HashSet::new();
    let mut servers = Vec::with_capacity(specs.len());

    for spec in specs {
        let id = spec.id.trim().to_owned();
        if id.is_empty()
            || !id.chars().all(|character| {
                character.is_ascii_lowercase()
                    || character.is_ascii_digit()
                    || matches!(character, '-' | '_' | '.')
            })
        {
            return Err(FeaturedServerError::Invalid {
                id,
                reason: "id must contain only lowercase ASCII letters, digits, '.', '-' or '_'"
                    .to_owned(),
            });
        }
        if !ids.insert(id.clone()) {
            return Err(FeaturedServerError::DuplicateId(id));
        }

        let name = spec.name.trim().to_owned();
        if name.is_empty() {
            return Err(FeaturedServerError::Invalid {
                id: id.clone(),
                reason: "name cannot be blank".to_owned(),
            });
        }
        let description = spec.description.trim().to_owned();
        if description.is_empty() {
            return Err(FeaturedServerError::Invalid {
                id: id.clone(),
                reason: "description cannot be blank".to_owned(),
            });
        }
        let address =
            ServerAddress::parse(&spec.address).map_err(|source| FeaturedServerError::Address {
                id: id.clone(),
                address: spec.address.clone(),
                source,
            })?;

        let version = spec
            .version
            .as_deref()
            .map(str::trim)
            .filter(|version| !version.is_empty())
            .map(str::to_owned);
        if version.is_none() && !spec.requires_mods {
            return Err(FeaturedServerError::Invalid {
                id: id.clone(),
                reason: "vanilla servers require a recommended version".to_owned(),
            });
        }
        if let Some(version) = version.as_deref()
            && !version.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_')
            })
        {
            return Err(FeaturedServerError::Invalid {
                id: id.clone(),
                reason: "version contains unsupported characters".to_owned(),
            });
        }

        let source_url = spec
            .source_url
            .as_deref()
            .map(str::trim)
            .filter(|url| !url.is_empty())
            .map(|url| {
                let parsed = Url::parse(url).map_err(|source| FeaturedServerError::Url {
                    id: id.clone(),
                    url: url.to_owned(),
                    source,
                })?;
                if parsed.scheme() != "https" || parsed.host_str().is_none() {
                    return Err(FeaturedServerError::Invalid {
                        id: id.clone(),
                        reason: "source_url must be an absolute https URL".to_owned(),
                    });
                }
                Ok(parsed.to_string())
            })
            .transpose()?;

        let tags = spec
            .tags
            .into_iter()
            .map(|tag| tag.trim().to_owned())
            .filter(|tag| !tag.is_empty())
            .collect();

        servers.push(FeaturedServer {
            id,
            name,
            address,
            version,
            version_note: spec
                .version_note
                .as_deref()
                .map(str::trim)
                .filter(|note| !note.is_empty())
                .map(str::to_owned),
            description,
            tags,
            requires_mods: spec.requires_mods,
            source_url,
        });
    }

    Ok(servers)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed() -> Vec<FeaturedServer> {
        parse_featured_servers(FEATURED_SERVERS_JSON).unwrap()
    }

    #[test]
    fn embedded_list_is_valid_and_unique() {
        let servers = parsed();
        assert!(!servers.is_empty());

        let ids = servers
            .iter()
            .map(|server| server.id.as_str())
            .collect::<HashSet<_>>();
        assert_eq!(ids.len(), servers.len());

        for server in &servers {
            if !server.requires_mods {
                assert!(server.version.is_some(), "{} has no version", server.id);
            }
        }
        assert!(servers.iter().any(|server| server.requires_mods));
        assert!(servers.iter().any(|server| server.can_join()));
    }

    #[test]
    fn embedded_list_parses_through_the_public_cache() {
        let servers = featured_servers().unwrap();
        assert!(!servers.is_empty());
        assert_eq!(servers.len(), parsed().len());
    }

    #[test]
    fn duplicate_ids_are_rejected() {
        let json = r#"[
            {"id":"a","name":"A","address":"a.example.com","version":"1.21.1","description":"a","source_url":"https://a.example.com/"},
            {"id":"a","name":"B","address":"b.example.com","version":"1.21.1","description":"b","source_url":"https://b.example.com/"}
        ]"#;
        assert!(matches!(
            parse_featured_servers(json),
            Err(FeaturedServerError::DuplicateId(id)) if id == "a"
        ));
    }

    #[test]
    fn invalid_address_is_rejected() {
        let json = r#"[
            {"id":"a","name":"A","address":"https://example.com/","version":"1.21.1","description":"a","source_url":"https://a.example.com/"}
        ]"#;
        assert!(matches!(
            parse_featured_servers(json),
            Err(FeaturedServerError::Address { id, .. }) if id == "a"
        ));
    }

    #[test]
    fn non_https_source_is_rejected() {
        let json = r#"[
            {"id":"a","name":"A","address":"a.example.com","version":"1.21.1","description":"a","source_url":"http://a.example.com/"}
        ]"#;
        assert!(matches!(
            parse_featured_servers(json),
            Err(FeaturedServerError::Invalid { id, .. }) if id == "a"
        ));
    }

    #[test]
    fn website_is_optional() {
        let json = r#"[
            {"id":"a","name":"A","address":"a.example.com","version":"1.21.1","description":"a"},
            {"id":"b","name":"B","address":"b.example.com","version":"1.21.1","description":"b","source_url":"  "}
        ]"#;
        let servers = parse_featured_servers(json).unwrap();
        assert!(servers.iter().all(|server| server.source_url.is_none()));
    }

    #[test]
    fn vanilla_server_requires_a_version() {
        let json = r#"[
            {"id":"a","name":"A","address":"a.example.com","description":"a","source_url":"https://a.example.com/"}
        ]"#;
        assert!(matches!(
            parse_featured_servers(json),
            Err(FeaturedServerError::Invalid { id, .. }) if id == "a"
        ));
    }
}
