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

    pub fn page_url(&self) -> String {
        format!("https://modrinth.com/{}/{}", self.project_type, self.slug)
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
