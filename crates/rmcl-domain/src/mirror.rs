use url::Url;

/// Built-in BMCLAPI mirror and the mappings it supports.
///
/// BMCLAPI keeps the official path layout for Mojang metadata and Java
/// runtimes, but mounts assets and Maven artifacts under `/assets` and
/// `/maven`. The mapping is deliberately limited to known hosts: never rewrite
/// an arbitrary URL from a version or loader manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirrorSource {
    base: Url,
}

impl MirrorSource {
    pub fn bmclapi() -> Self {
        Self::new(
            Url::parse("https://bmclapi2.bangbang93.com/")
                .expect("the built-in BMCLAPI URL is valid"),
        )
    }

    pub fn new(base: Url) -> Self {
        let mut base = base;
        if !base.path().ends_with('/') {
            let path = format!("{}/", base.path());
            base.set_path(&path);
        }
        Self { base }
    }

    pub fn base(&self) -> &Url {
        &self.base
    }

    /// A small metadata document used to measure mirror availability.
    pub fn probe_url(&self) -> Url {
        self.base
            .join("mc/game/version_manifest_v2.json")
            .expect("the mirror base is a valid URL base")
    }

    /// Returns the mirror URL for an official URL, if this mirror serves it.
    ///
    /// Every mapping below was verified against the live BMCLAPI service:
    /// version manifests, version JSON, client JARs, assets, libraries,
    /// Fabric metadata/Maven, Forge Maven and NeoForge release artifacts.
    /// Quilt and NeoForge API endpoints are intentionally absent because the
    /// mirror did not serve them when checked.
    pub fn mirror_url(&self, official: &Url) -> Option<Url> {
        let host = official.host_str()?;
        match host {
            "launchermeta.mojang.com"
            | "launcher.mojang.com"
            | "piston-meta.mojang.com"
            | "piston-data.mojang.com" => self.with_host(official),
            "resources.download.minecraft.net" => self.with_path(official, "/assets", None),
            "libraries.minecraft.net" | "maven.fabricmc.net" | "maven.minecraftforge.net" => {
                self.with_path(official, "/maven", None)
            }
            "files.minecraftforge.net" => {
                if official.path().starts_with("/maven/") {
                    self.with_host(official)
                } else {
                    self.with_path(official, "/maven", None)
                }
            }
            "maven.neoforged.net" => {
                if official.path().starts_with("/releases/") {
                    self.with_path(official, "/maven", Some("/releases"))
                } else {
                    None
                }
            }
            "meta.fabricmc.net" => self.with_path(official, "/fabric-meta", None),
            "authlib-injector.yushi.moe" => {
                self.with_path(official, "/mirrors/authlib-injector", None)
            }
            _ => None,
        }
    }

    /// Adds the mirror as the first candidate when it serves `official`.
    pub fn add_candidates(&self, sources: &mut Vec<Url>) {
        let Some(official) = sources.first().cloned() else {
            return;
        };
        let Some(mirror) = self.mirror_url(&official) else {
            return;
        };
        if !sources.contains(&mirror) {
            sources.insert(0, mirror);
        }
    }

    /// Mirror first, official second. Falls back to the official URL when this
    /// mirror has no mapping for it.
    pub fn candidates(&self, official: &Url) -> Vec<Url> {
        let mut sources = vec![official.clone()];
        self.add_candidates(&mut sources);
        sources
    }

    fn with_host(&self, official: &Url) -> Option<Url> {
        let mut url = official.clone();
        url.set_scheme("https").ok()?;
        url.set_host(Some(self.base.host_str()?)).ok()?;
        Some(url)
    }

    fn with_path(&self, official: &Url, prefix: &str, strip: Option<&str>) -> Option<Url> {
        let path = strip
            .and_then(|prefix| official.path().strip_prefix(prefix))
            .unwrap_or_else(|| official.path());
        let mut url = official.clone();
        url.set_scheme("https").ok()?;
        url.set_host(Some(self.base.host_str()?)).ok()?;
        url.set_path(&format!("{prefix}{path}"));
        Some(url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn official(value: &str) -> Url {
        Url::parse(value).unwrap()
    }

    #[test]
    fn maps_mojang_metadata_and_client_urls_by_host() {
        let mirror = MirrorSource::bmclapi();
        for value in [
            "https://launchermeta.mojang.com/mc/game/version_manifest_v2.json",
            "https://launcher.mojang.com/v1/objects/abc/client.jar",
            "https://piston-meta.mojang.com/v1/packages/abc/1.21.1.json",
            "https://piston-data.mojang.com/v1/objects/abc/client.jar",
        ] {
            let mapped = mirror.mirror_url(&official(value)).unwrap();
            assert_eq!(mapped.host_str(), Some("bmclapi2.bangbang93.com"));
            assert_eq!(mapped.path(), official(value).path());
        }
    }

    #[test]
    fn maps_assets_libraries_and_loader_hosts_to_their_mount_points() {
        let mirror = MirrorSource::bmclapi();
        let cases = [
            (
                "https://resources.download.minecraft.net/3f/3fabc",
                "/assets/3f/3fabc",
            ),
            (
                "https://libraries.minecraft.net/org/example/lib.jar",
                "/maven/org/example/lib.jar",
            ),
            (
                "https://maven.fabricmc.net/net/fabricmc/loader.jar",
                "/maven/net/fabricmc/loader.jar",
            ),
            (
                "https://maven.minecraftforge.net/net/minecraftforge/forge/installer.jar",
                "/maven/net/minecraftforge/forge/installer.jar",
            ),
            (
                "https://files.minecraftforge.net/net/minecraftforge/forge/maven-metadata.json",
                "/maven/net/minecraftforge/forge/maven-metadata.json",
            ),
            (
                "https://files.minecraftforge.net/maven/net/minecraftforge/forge/installer.jar",
                "/maven/net/minecraftforge/forge/installer.jar",
            ),
            (
                "https://maven.neoforged.net/releases/net/neoforged/neoforge/installer.jar",
                "/maven/net/neoforged/neoforge/installer.jar",
            ),
            (
                "https://meta.fabricmc.net/v2/versions/loader/1.21.1",
                "/fabric-meta/v2/versions/loader/1.21.1",
            ),
            (
                "https://authlib-injector.yushi.moe/artifact/latest.json",
                "/mirrors/authlib-injector/artifact/latest.json",
            ),
        ];
        for (source, path) in cases {
            let mapped = mirror.mirror_url(&official(source)).unwrap();
            assert_eq!(mapped.host_str(), Some("bmclapi2.bangbang93.com"));
            assert_eq!(mapped.path(), path, "source: {source}");
        }
    }

    #[test]
    fn leaves_quilt_and_neoforge_api_on_the_official_source() {
        let mirror = MirrorSource::bmclapi();
        for value in [
            "https://meta.quiltmc.org/v3/versions/loader/1.21.1",
            "https://maven.quiltmc.org/repository/release/org/quiltmc/loader.jar",
            "https://maven.neoforged.net/api/maven/versions/releases/net/neoforged/neoforge",
        ] {
            assert!(mirror.mirror_url(&official(value)).is_none(), "{value}");
        }
    }

    #[test]
    fn leaves_unknown_hosts_on_the_official_source() {
        let mirror = MirrorSource::bmclapi();
        let url = official("https://api.modrinth.com/v2/project/sodium");
        assert!(mirror.mirror_url(&url).is_none());
        assert_eq!(mirror.candidates(&url), vec![url]);
    }

    #[test]
    fn adds_the_mirror_before_the_official_candidate_once() {
        let mirror = MirrorSource::bmclapi();
        let official = official("https://libraries.minecraft.net/org/example/lib.jar");
        let mut sources = vec![official.clone()];
        mirror.add_candidates(&mut sources);
        mirror.add_candidates(&mut sources);

        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].host_str(), Some("bmclapi2.bangbang93.com"));
        assert_eq!(sources[1], official);
    }
}
