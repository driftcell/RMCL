use std::{
    sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
    time::{Duration, Instant},
};

use reqwest::Client;
use rmcl_domain::MirrorSource;
use url::Url;

/// User-visible download source policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MirrorPreference {
    /// Probe the mirror and use it when it is reachable and fast.
    #[default]
    Auto,
    /// Never use a mirror.
    Official,
    /// Prefer the mirror and fall back to official per request.
    Mirror,
}

impl MirrorPreference {
    fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Official,
            2 => Self::Mirror,
            _ => Self::Auto,
        }
    }

    fn as_u8(self) -> u8 {
        match self {
            Self::Auto => 0,
            Self::Official => 1,
            Self::Mirror => 2,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirrorProbe {
    pub ok: bool,
    pub latency: Option<Duration>,
    pub error: Option<String>,
}

/// Fast mirror selection shared by installers, metadata clients, and the UI.
///
/// `Auto` starts disabled until [`Self::probe`] succeeds. `Mirror` starts
/// enabled and stays enabled even when the probe fails, because the user asked
/// for it; every generated request still carries the official URL as a
/// fallback.
pub struct MirrorHealth {
    preference: AtomicU8,
    enabled: AtomicBool,
    latency_ms: AtomicU64,
    mirror: MirrorSource,
}

/// A mirror is considered usable when the probe completes within this budget.
pub const MAX_PROBE_LATENCY: Duration = Duration::from_millis(1500);

impl MirrorHealth {
    pub fn new(preference: MirrorPreference) -> Self {
        Self {
            preference: AtomicU8::new(preference.as_u8()),
            enabled: AtomicBool::new(preference == MirrorPreference::Mirror),
            latency_ms: AtomicU64::new(0),
            mirror: MirrorSource::bmclapi(),
        }
    }

    pub fn with_mirror(mut self, mirror: MirrorSource) -> Self {
        self.mirror = mirror;
        self
    }

    pub fn preference(&self) -> MirrorPreference {
        MirrorPreference::from_u8(self.preference.load(Ordering::Relaxed))
    }

    pub fn set_preference(&self, preference: MirrorPreference) {
        self.preference.store(preference.as_u8(), Ordering::Relaxed);
        self.enabled
            .store(preference == MirrorPreference::Mirror, Ordering::Relaxed);
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    pub fn latency(&self) -> Option<Duration> {
        let millis = self.latency_ms.load(Ordering::Relaxed);
        (millis > 0).then(|| Duration::from_millis(millis))
    }

    pub fn mirror(&self) -> &MirrorSource {
        &self.mirror
    }

    /// Mirror candidate first, then the official URL. Unknown hosts only keep
    /// the official URL.
    pub fn sources_for(&self, official: &Url) -> Vec<Url> {
        if self.is_enabled() {
            self.mirror.candidates(official)
        } else {
            vec![official.clone()]
        }
    }

    pub fn add_candidates(&self, sources: &mut Vec<Url>) {
        if self.is_enabled() {
            self.mirror.add_candidates(sources);
        }
    }

    /// Measures the mirror metadata endpoint. `Auto` enables the mirror only
    /// when the response succeeds within [`MAX_PROBE_LATENCY`].
    pub async fn probe(&self, client: &Client, timeout: Duration) -> MirrorProbe {
        if self.preference() == MirrorPreference::Official {
            self.enabled.store(false, Ordering::Relaxed);
            return MirrorProbe {
                ok: false,
                latency: None,
                error: Some("mirror disabled by configuration".to_owned()),
            };
        }

        let started = Instant::now();
        let result = tokio::time::timeout(timeout, async {
            let response = client
                .get(self.mirror.probe_url())
                .send()
                .await
                .map_err(|error| error.to_string())?;
            let status = response.status();
            if !status.is_success() {
                return Err(format!("HTTP {}", status.as_u16()));
            }
            let body = response.bytes().await.map_err(|error| error.to_string())?;
            if body.is_empty() {
                return Err("empty probe response".to_owned());
            }
            Ok(())
        })
        .await;

        match result {
            Ok(Ok(())) => {
                let latency = started.elapsed();
                self.latency_ms
                    .store(latency.as_millis() as u64, Ordering::Relaxed);
                let usable = latency <= MAX_PROBE_LATENCY;
                match self.preference() {
                    MirrorPreference::Auto => self.enabled.store(usable, Ordering::Relaxed),
                    MirrorPreference::Mirror => self.enabled.store(true, Ordering::Relaxed),
                    MirrorPreference::Official => self.enabled.store(false, Ordering::Relaxed),
                }
                MirrorProbe {
                    ok: true,
                    latency: Some(latency),
                    error: None,
                }
            }
            Ok(Err(error)) => {
                if self.preference() == MirrorPreference::Auto {
                    self.enabled.store(false, Ordering::Relaxed);
                }
                MirrorProbe {
                    ok: false,
                    latency: None,
                    error: Some(error),
                }
            }
            Err(_) => {
                if self.preference() == MirrorPreference::Auto {
                    self.enabled.store(false, Ordering::Relaxed);
                }
                MirrorProbe {
                    ok: false,
                    latency: None,
                    error: Some(format!("probe timed out after {} ms", timeout.as_millis())),
                }
            }
        }
    }

    /// Builds a short-lived client and probes without requiring the caller to
    /// depend on `reqwest`.
    pub async fn probe_default(&self, timeout: Duration) -> MirrorProbe {
        let client = match Client::builder()
            .user_agent(concat!("RMCL/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(5))
            .timeout(timeout + Duration::from_secs(1))
            .build()
        {
            Ok(client) => client,
            Err(error) => {
                if self.preference() == MirrorPreference::Auto {
                    self.enabled.store(false, Ordering::Relaxed);
                }
                return MirrorProbe {
                    ok: false,
                    latency: None,
                    error: Some(error.to_string()),
                };
            }
        };
        self.probe(&client, timeout).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirror_preference_starts_enabled_only_when_forced() {
        assert!(!MirrorHealth::new(MirrorPreference::Auto).is_enabled());
        assert!(!MirrorHealth::new(MirrorPreference::Official).is_enabled());
        assert!(MirrorHealth::new(MirrorPreference::Mirror).is_enabled());
    }

    #[test]
    fn sources_fall_back_to_official_when_disabled() {
        let health = MirrorHealth::new(MirrorPreference::Official);
        let official =
            Url::parse("https://piston-meta.mojang.com/v1/packages/abc/1.21.1.json").unwrap();
        assert_eq!(health.sources_for(&official), vec![official.clone()]);

        let health = MirrorHealth::new(MirrorPreference::Mirror);
        let sources = health.sources_for(&official);
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].host_str(), Some("bmclapi2.bangbang93.com"));
        assert_eq!(sources[1], official);
    }

    #[test]
    fn unknown_hosts_never_gain_a_mirror_candidate() {
        let health = MirrorHealth::new(MirrorPreference::Mirror);
        let official = Url::parse("https://api.modrinth.com/v2/project/sodium").unwrap();
        assert_eq!(health.sources_for(&official), vec![official]);
    }

    #[test]
    fn switching_to_mirror_enables_candidates_and_switching_back_disables_them() {
        let health = MirrorHealth::new(MirrorPreference::Auto);
        let official = Url::parse("https://libraries.minecraft.net/org/example/lib.jar").unwrap();
        assert_eq!(health.sources_for(&official).len(), 1);

        health.set_preference(MirrorPreference::Mirror);
        assert_eq!(health.sources_for(&official).len(), 2);

        health.set_preference(MirrorPreference::Official);
        assert_eq!(health.sources_for(&official), vec![official]);
    }

    #[tokio::test]
    async fn probe_enables_auto_mode_for_a_reachable_mirror() {
        use axum::{Router, routing::get};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        let server = tokio::spawn(async move {
            let router =
                Router::new().route("/mc/game/version_manifest_v2.json", get(|| async { "{}" }));
            axum::serve(listener, router).await.unwrap();
        });

        let health = MirrorHealth::new(MirrorPreference::Auto).with_mirror(MirrorSource::new(base));
        let probe = health.probe(&Client::new(), Duration::from_secs(1)).await;

        assert!(probe.ok);
        assert!(health.is_enabled());
        server.abort();
    }

    #[tokio::test]
    async fn probe_keeps_auto_mode_on_official_for_an_unreachable_mirror() {
        use axum::{Router, http::StatusCode, routing::get};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        let server = tokio::spawn(async move {
            let router = Router::new().route(
                "/mc/game/version_manifest_v2.json",
                get(|| async { StatusCode::SERVICE_UNAVAILABLE }),
            );
            axum::serve(listener, router).await.unwrap();
        });

        let health = MirrorHealth::new(MirrorPreference::Auto).with_mirror(MirrorSource::new(base));
        let probe = health.probe(&Client::new(), Duration::from_secs(1)).await;

        assert!(!probe.ok);
        assert!(!health.is_enabled());
        server.abort();
    }
}
