use std::path::PathBuf;

use rmcl_instance::RepositoryPath;

use crate::{ContentKind, kind::strip_disabled_suffix};

/// One local content entry discovered under an instance content directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalContent {
    pub kind: ContentKind,
    /// Safe path relative to the kind's content directory.
    pub relative_path: RepositoryPath,
    pub path: PathBuf,
    pub size: Option<u64>,
    pub enabled: bool,
    pub is_directory: bool,
}

impl LocalContent {
    pub fn file_name(&self) -> &str {
        self.relative_path
            .as_str()
            .rsplit('/')
            .next()
            .unwrap_or_else(|| self.relative_path.as_str())
    }

    /// File name without the RMCL `.disabled` suffix.
    pub fn display_name(&self) -> &str {
        strip_disabled_suffix(self.file_name()).0
    }

    pub fn can_toggle(&self) -> bool {
        matches!(self.kind, ContentKind::Mod | ContentKind::ResourcePack)
    }

    pub fn is_disabled(&self) -> bool {
        !self.enabled
    }
}

/// A non-fatal issue encountered while scanning a content directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentWarning {
    Symlink { path: PathBuf },
    InvalidName { path: PathBuf, reason: String },
    Unreadable { path: PathBuf, message: String },
    Options { path: PathBuf, message: String },
    Scan { kind: ContentKind, message: String },
}

impl ContentWarning {
    pub fn message(&self) -> String {
        match self {
            Self::Symlink { path } => format!("已跳过符号链接 {}", path.display()),
            Self::InvalidName { path, reason } => {
                format!("已跳过无效名称 {}：{reason}", path.display())
            }
            Self::Unreadable { path, message } => {
                format!("无法读取 {}：{message}", path.display())
            }
            Self::Options { path, message } => {
                format!("{} 无法解析：{message}", path.display())
            }
            Self::Scan { kind, message } => {
                format!("扫描 {} 失败：{message}", kind.label())
            }
        }
    }
}

/// Result of scanning one content kind.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContentScan {
    pub items: Vec<LocalContent>,
    pub warnings: Vec<ContentWarning>,
}

/// All locally managed content of one instance.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContentLibrary {
    pub items: Vec<LocalContent>,
    pub warnings: Vec<ContentWarning>,
}

impl ContentLibrary {
    pub fn items_for(&self, kind: ContentKind) -> impl Iterator<Item = &LocalContent> {
        self.items.iter().filter(move |item| item.kind == kind)
    }

    pub fn count(&self, kind: ContentKind) -> usize {
        self.items.iter().filter(|item| item.kind == kind).count()
    }

    pub fn enabled_count(&self, kind: ContentKind) -> usize {
        self.items
            .iter()
            .filter(|item| item.kind == kind && item.enabled)
            .count()
    }
}
