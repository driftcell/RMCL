use std::collections::BTreeMap;

use relative_path::{Component, RelativePath, RelativePathBuf};
use rmcl_instance::RepositoryPath;

use crate::{JavaError, MojangFileType};

#[derive(Default)]
pub(crate) struct PathRegistry(BTreeMap<String, MojangFileType>);

impl PathRegistry {
    pub(crate) fn insert(
        &mut self,
        path: &RepositoryPath,
        kind: MojangFileType,
    ) -> Result<(), JavaError> {
        let key = path.as_str().to_lowercase();
        for (existing, existing_kind) in &self.0 {
            if key == *existing || paths_conflict(&key, existing, *existing_kind, kind) {
                return Err(JavaError::PathConflict {
                    path: path.to_string(),
                    existing: existing.clone(),
                });
            }
        }
        self.0.insert(key, kind);
        Ok(())
    }
}

fn paths_conflict(
    left: &str,
    right: &str,
    left_kind: MojangFileType,
    right_kind: MojangFileType,
) -> bool {
    (right_kind != MojangFileType::Directory && is_parent(left, right))
        || (left_kind != MojangFileType::Directory && is_parent(right, left))
}

fn is_parent(parent: &str, child: &str) -> bool {
    child.starts_with(&(parent.to_owned() + "/"))
}

#[derive(Debug, Clone)]
pub(crate) struct NormalizedLink {
    pub(crate) source: RepositoryPath,
    pub(crate) target: RepositoryPath,
    pub(crate) relative: RelativePathBuf,
}

pub(crate) fn normalize_link_target(
    source: &RepositoryPath,
    target: &str,
) -> Result<NormalizedLink, JavaError> {
    if target.is_empty()
        || target.starts_with('/')
        || target.contains('\\')
        || target.contains(':')
        || target.chars().any(char::is_control)
    {
        return Err(JavaError::InvalidLink {
            path: source.to_string(),
            reason:
                "target must be a relative path without drive, backslash, or control characters"
                    .to_owned(),
        });
    }

    let parent = source
        .as_str()
        .rsplit_once('/')
        .map(|(parent, _)| parent)
        .unwrap_or("");
    let joined = if parent.is_empty() {
        target.to_owned()
    } else {
        format!("{parent}/{target}")
    };
    let normalized = RelativePath::new(&joined).normalize();
    if normalized
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(JavaError::InvalidLink {
            path: source.to_string(),
            reason: "target escapes the runtime root".to_owned(),
        });
    }
    let target_path = RepositoryPath::new(normalized.as_str())?;
    if target_path == *source {
        return Err(JavaError::InvalidLink {
            path: source.to_string(),
            reason: "link points to itself".to_owned(),
        });
    }
    let relative = relative_link_path(source, &target_path)?;
    Ok(NormalizedLink {
        source: source.clone(),
        target: target_path,
        relative,
    })
}

fn relative_link_path(
    source: &RepositoryPath,
    target: &RepositoryPath,
) -> Result<RelativePathBuf, JavaError> {
    let source_parts: Vec<_> = source.as_str().split('/').collect();
    let source_parent = &source_parts[..source_parts.len() - 1];
    let target_parts: Vec<_> = target.as_str().split('/').collect();
    let common = source_parent
        .iter()
        .zip(target_parts.iter())
        .take_while(|(left, right)| left == right)
        .count();
    let mut parts = Vec::new();
    parts.extend(std::iter::repeat_n("..", source_parent.len() - common));
    parts.extend(target_parts[common..].iter().copied());
    if parts.is_empty() {
        return Err(JavaError::InvalidLink {
            path: source.to_string(),
            reason: "link points to itself".to_owned(),
        });
    }
    Ok(RelativePathBuf::from(parts.join("/")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_relative_links_inside_runtime_root() {
        let source = RepositoryPath::new("legal/readme").unwrap();
        let link = normalize_link_target(&source, "../java.base/readme").unwrap();

        assert_eq!(link.target.as_str(), "java.base/readme");
        assert_eq!(link.relative.as_str(), "../java.base/readme");
    }

    #[test]
    fn rejects_escaping_or_absolute_links() {
        let source = RepositoryPath::new("legal/readme").unwrap();
        for target in ["/etc/passwd", "C:/escape", "../../../escape", "a\\b"] {
            assert!(
                normalize_link_target(&source, target).is_err(),
                "accepted {target}"
            );
        }
    }

    #[test]
    fn rejects_self_links() {
        let source = RepositoryPath::new("legal/readme").unwrap();
        assert!(normalize_link_target(&source, "readme").is_err());
    }

    #[test]
    fn allows_directory_parents_for_files_and_links() {
        let mut registry = PathRegistry::default();
        registry
            .insert(
                &RepositoryPath::new("legal").unwrap(),
                MojangFileType::Directory,
            )
            .unwrap();
        registry
            .insert(
                &RepositoryPath::new("legal/readme").unwrap(),
                MojangFileType::Link,
            )
            .unwrap();
        registry
            .insert(
                &RepositoryPath::new("legal/info").unwrap(),
                MojangFileType::File,
            )
            .unwrap();
    }

    #[test]
    fn rejects_file_directory_conflicts() {
        let mut registry = PathRegistry::default();
        registry
            .insert(&RepositoryPath::new("a").unwrap(), MojangFileType::File)
            .unwrap();
        let error = registry
            .insert(&RepositoryPath::new("a/b").unwrap(), MojangFileType::File)
            .unwrap_err();
        assert!(matches!(error, JavaError::PathConflict { .. }));
    }
}
