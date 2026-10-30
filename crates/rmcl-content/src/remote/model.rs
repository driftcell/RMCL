use std::{fmt, str::FromStr};

use rmcl_domain::Sha1Hash;
use url::Url;

use crate::ContentKind;

/// Remote content provider backing a project or version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContentSource {
    Modrinth,
}

impl ContentSource {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Modrinth => "Modrinth",
        }
    }
}

impl fmt::Display for ContentSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.label())
    }
}

/// Sort order accepted by remote content search.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RemoteSort {
    #[default]
    Relevance,
    Downloads,
    Follows,
    Newest,
    Updated,
}

impl RemoteSort {
    pub const ALL: [Self; 5] = [
        Self::Relevance,
        Self::Downloads,
        Self::Follows,
        Self::Newest,
        Self::Updated,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Relevance => "相关度",
            Self::Downloads => "下载量",
            Self::Follows => "关注数",
            Self::Newest => "最新发布",
            Self::Updated => "最近更新",
        }
    }

    pub const fn modrinth_index(self) -> &'static str {
        match self {
            Self::Relevance => "relevance",
            Self::Downloads => "downloads",
            Self::Follows => "follows",
            Self::Newest => "newest",
            Self::Updated => "updated",
        }
    }
}

/// Search request for one remote content kind and instance context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteSearchQuery {
    pub source: ContentSource,
    pub kind: ContentKind,
    pub query: String,
    pub game_version: Option<String>,
    pub loader: Option<String>,
    pub offset: usize,
    pub limit: usize,
    pub sort: RemoteSort,
}

impl RemoteSearchQuery {
    pub fn new(kind: ContentKind) -> Self {
        Self {
            source: ContentSource::Modrinth,
            kind,
            query: String::new(),
            game_version: None,
            loader: None,
            offset: 0,
            limit: 20,
            sort: RemoteSort::Relevance,
        }
    }
}

/// One page of remote project search results.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteSearchPage {
    pub projects: Vec<RemoteProject>,
    pub total_hits: u64,
    pub offset: usize,
    pub limit: usize,
}

/// Provider-neutral project summary used by search and version pages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteProject {
    pub source: ContentSource,
    pub id: String,
    pub slug: String,
    pub title: String,
    pub description: String,
    pub author: String,
    pub icon_url: Option<String>,
    pub downloads: u64,
    pub follows: u64,
    pub categories: Vec<String>,
    pub game_versions: Vec<String>,
    pub loaders: Vec<String>,
    pub project_type: String,
    pub updated: Option<String>,
    /// Provider-hosted gallery thumbnails, in display order.
    pub gallery: Vec<String>,
    /// The provider-selected thumbnail to use for project cards.
    pub featured_gallery: Option<String>,
}

impl RemoteProject {
    pub fn kind(&self) -> Option<ContentKind> {
        match self.project_type.as_str() {
            "mod" => Some(ContentKind::Mod),
            "resourcepack" => Some(ContentKind::ResourcePack),
            "shader" => Some(ContentKind::ShaderPack),
            _ => None,
        }
    }

    /// Returns the instance loader to send when listing versions of this project.
    ///
    /// Only mods are loader-specific. Resource packs advertise `minecraft`, while
    /// shaders advertise `iris`/`optifine`; filtering those by the instance's mod
    /// loader would hide every version on Fabric/Quilt/Forge/NeoForge instances.
    pub fn version_loader_filter(&self, instance_loader: Option<&str>) -> Option<String> {
        (self.kind() == Some(ContentKind::Mod))
            .then(|| instance_loader.map(str::to_owned))
            .flatten()
    }

    pub fn page_url(&self) -> String {
        format!("https://modrinth.com/{}/{}", self.project_type, self.slug)
    }

    /// Returns the most representative available gallery thumbnail.
    pub fn preview_url(&self) -> Option<&str> {
        self.featured_gallery
            .as_deref()
            .or_else(|| self.gallery.first().map(String::as_str))
    }

    /// Returns gallery thumbnails with the featured image first and duplicates
    /// removed. Search results may list the featured image separately.
    pub fn gallery_previews(&self) -> Vec<&str> {
        let mut images = Vec::new();
        if let Some(featured) = self.featured_gallery.as_deref() {
            images.push(featured);
        }
        for image in &self.gallery {
            if !images.contains(&image.as_str()) {
                images.push(image);
            }
        }
        images
    }
}

/// One remote project version with its downloadable files and dependencies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteVersion {
    pub source: ContentSource,
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub version_number: String,
    pub changelog: Option<String>,
    pub published: Option<String>,
    pub version_type: String,
    pub downloads: u64,
    pub game_versions: Vec<String>,
    pub loaders: Vec<String>,
    pub files: Vec<RemoteFile>,
    pub dependencies: Vec<RemoteDependency>,
}

impl RemoteVersion {
    /// The preferred installable file: the primary file when it is accepted by
    /// the content kind, otherwise the first accepted file.
    pub fn installable_file(&self, kind: ContentKind) -> Option<&RemoteFile> {
        self.files
            .iter()
            .find(|file| file.primary && kind.accepts_file_name(&file.filename))
            .or_else(|| {
                self.files
                    .iter()
                    .find(|file| kind.accepts_file_name(&file.filename))
            })
    }

    pub fn required_dependencies(&self) -> impl Iterator<Item = &RemoteDependency> {
        self.dependencies
            .iter()
            .filter(|dependency| dependency.kind == RemoteDependencyKind::Required)
    }

    /// 3 = release, 2 = beta, 1 = alpha, 0 = unknown.
    pub fn stability_rank(&self) -> u8 {
        match self.version_type.as_str() {
            "release" => 3,
            "beta" => 2,
            "alpha" => 1,
            _ => 0,
        }
    }
}

/// Picks the newest installable version with the best stability.
///
/// Modrinth returns versions newest-first. Only versions with a usable file and
/// a SHA-1 are considered, so the recommendation never points at an install
/// action that the UI would immediately reject.
pub fn recommended_version(
    versions: &[RemoteVersion],
    kind: ContentKind,
) -> Option<&RemoteVersion> {
    for rank in [3, 2, 1, 0] {
        if let Some(version) = versions.iter().find(|version| {
            version.stability_rank() == rank
                && version
                    .installable_file(kind)
                    .is_some_and(|file| file.sha1.is_some())
        }) {
            return Some(version);
        }
    }
    None
}

/// One downloadable file belonging to a remote version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteFile {
    pub filename: String,
    pub url: Url,
    pub size: Option<u64>,
    pub sha1: Option<Sha1Hash>,
    pub primary: bool,
}

/// Remote dependency relation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteDependencyKind {
    Required,
    Optional,
    Incompatible,
    Embedded,
    Unknown,
}

impl RemoteDependencyKind {
    pub fn from_api(value: &str) -> Self {
        match value {
            "required" => Self::Required,
            "optional" => Self::Optional,
            "incompatible" => Self::Incompatible,
            "embedded" => Self::Embedded,
            _ => Self::Unknown,
        }
    }
}

/// A project version dependency advertised by a provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteDependency {
    pub version_id: Option<String>,
    pub project_id: Option<String>,
    pub file_name: Option<String>,
    pub kind: RemoteDependencyKind,
}

pub(crate) fn parse_sha1(value: &str) -> Option<Sha1Hash> {
    Sha1Hash::from_str(value).ok()
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    fn project(project_type: &str) -> RemoteProject {
        RemoteProject {
            source: ContentSource::Modrinth,
            id: "project".to_owned(),
            slug: "project".to_owned(),
            title: "Project".to_owned(),
            description: String::new(),
            author: String::new(),
            icon_url: None,
            downloads: 0,
            follows: 0,
            categories: Vec::new(),
            game_versions: Vec::new(),
            loaders: Vec::new(),
            project_type: project_type.to_owned(),
            updated: None,
            gallery: Vec::new(),
            featured_gallery: None,
        }
    }

    fn version(id: &str, version_type: &str, filename: &str, sha1: Option<&str>) -> RemoteVersion {
        RemoteVersion {
            source: ContentSource::Modrinth,
            id: id.to_owned(),
            project_id: "project".to_owned(),
            name: id.to_owned(),
            version_number: "1.0.0".to_owned(),
            changelog: None,
            published: Some("2026-01-01T00:00:00Z".to_owned()),
            version_type: version_type.to_owned(),
            downloads: 0,
            game_versions: vec!["1.21.1".to_owned()],
            loaders: vec!["fabric".to_owned()],
            files: vec![RemoteFile {
                filename: filename.to_owned(),
                url: Url::parse("https://example.invalid/mod.jar").unwrap(),
                size: Some(1),
                sha1: sha1.map(|value| Sha1Hash::from_str(value).unwrap()),
                primary: true,
            }],
            dependencies: Vec::new(),
        }
    }

    #[test]
    fn version_loader_filter_only_applies_to_mods() {
        assert_eq!(
            project("mod").version_loader_filter(Some("fabric")),
            Some("fabric".to_owned())
        );
        assert_eq!(
            project("resourcepack").version_loader_filter(Some("fabric")),
            None
        );
        assert_eq!(
            project("shader").version_loader_filter(Some("fabric")),
            None
        );
        assert_eq!(project("mod").version_loader_filter(None), None);
    }

    #[test]
    fn preview_prefers_the_featured_gallery_image() {
        let mut project = project("shader");
        assert_eq!(project.preview_url(), None);
        project.gallery = vec!["https://cdn.example.com/first.webp".to_owned()];
        assert_eq!(
            project.preview_url(),
            Some("https://cdn.example.com/first.webp")
        );
        project.featured_gallery = Some("https://cdn.example.com/featured.webp".to_owned());
        project
            .gallery
            .push("https://cdn.example.com/featured.webp".to_owned());
        assert_eq!(
            project.preview_url(),
            Some("https://cdn.example.com/featured.webp")
        );
        assert_eq!(
            project.gallery_previews(),
            [
                "https://cdn.example.com/featured.webp",
                "https://cdn.example.com/first.webp"
            ]
        );
    }

    #[test]
    fn recommendation_prefers_the_newest_verified_release() {
        let versions = vec![
            version(
                "beta",
                "beta",
                "beta.jar",
                Some("0000000000000000000000000000000000000001"),
            ),
            version(
                "release",
                "release",
                "release.jar",
                Some("0000000000000000000000000000000000000002"),
            ),
            version(
                "alpha",
                "alpha",
                "alpha.jar",
                Some("0000000000000000000000000000000000000003"),
            ),
        ];
        assert_eq!(
            recommended_version(&versions, ContentKind::Mod).map(|version| version.id.as_str()),
            Some("release")
        );
    }

    #[test]
    fn recommendation_skips_versions_without_a_sha1() {
        let versions = vec![
            version("unverified", "release", "unverified.jar", None),
            version(
                "verified",
                "beta",
                "verified.jar",
                Some("0000000000000000000000000000000000000004"),
            ),
        ];
        assert_eq!(
            recommended_version(&versions, ContentKind::Mod).map(|version| version.id.as_str()),
            Some("verified")
        );
    }
}
