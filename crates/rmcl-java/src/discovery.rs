use std::{collections::HashSet, path::PathBuf};

use crate::{JavaError, JavaRuntime, JavaRuntimeSource, ProbeConfig, probe_with_config};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JavaCandidate {
    pub executable: PathBuf,
    pub source: JavaRuntimeSource,
}

#[derive(Debug, Default)]
pub struct JavaDiscovery {
    pub runtimes: Vec<JavaRuntime>,
    pub failures: Vec<JavaProbeFailure>,
}

#[derive(Debug)]
pub struct JavaProbeFailure {
    pub executable: PathBuf,
    pub error: JavaError,
}

#[derive(Debug, Clone)]
pub struct JavaDetector {
    managed_roots: Vec<PathBuf>,
    disabled: HashSet<PathBuf>,
    include_java_home: bool,
    include_path: bool,
    probe_config: ProbeConfig,
}

impl JavaDetector {
    pub fn new(managed_roots: Vec<PathBuf>) -> Self {
        Self {
            managed_roots,
            disabled: HashSet::new(),
            include_java_home: true,
            include_path: true,
            probe_config: ProbeConfig::default(),
        }
    }

    pub fn with_java_home(mut self, enabled: bool) -> Self {
        self.include_java_home = enabled;
        self
    }

    pub fn with_path(mut self, enabled: bool) -> Self {
        self.include_path = enabled;
        self
    }

    pub fn with_disabled_paths(mut self, paths: impl IntoIterator<Item = PathBuf>) -> Self {
        self.disabled = paths
            .into_iter()
            .map(|path| std::fs::canonicalize(&path).unwrap_or(path))
            .collect();
        self
    }

    pub fn with_probe_config(mut self, config: ProbeConfig) -> Self {
        self.probe_config = config;
        self
    }

    pub fn candidates(&self) -> Vec<JavaCandidate> {
        let mut candidates = Vec::new();
        for root in &self.managed_roots {
            discover_managed_root(root, &mut candidates);
        }

        if self.include_java_home
            && let Some(home) = std::env::var_os("JAVA_HOME")
        {
            push_candidate(
                &mut candidates,
                PathBuf::from(home).join("bin").join(java_binary_name()),
                JavaRuntimeSource::JavaHome,
            );
        }
        if self.include_path
            && let Ok(executable) = which::which("java")
        {
            push_candidate(&mut candidates, executable, JavaRuntimeSource::System);
        }

        let mut seen = HashSet::new();
        candidates.retain(|candidate| {
            !self.disabled.contains(&candidate.executable)
                && seen.insert(candidate.executable.clone())
        });
        candidates
    }

    pub async fn discover(&self) -> JavaDiscovery {
        let mut discovery = JavaDiscovery::default();
        for candidate in self.candidates() {
            match probe_with_config(
                candidate.executable.clone(),
                candidate.source,
                self.probe_config.clone(),
            )
            .await
            {
                Ok(runtime) => discovery.runtimes.push(runtime),
                Err(error) => discovery.failures.push(JavaProbeFailure {
                    executable: candidate.executable,
                    error,
                }),
            }
        }
        discovery
    }
}

fn discover_managed_root(root: &std::path::Path, candidates: &mut Vec<JavaCandidate>) {
    push_candidate(
        candidates,
        root.join("bin").join(java_binary_name()),
        JavaRuntimeSource::Managed,
    );
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    let mut homes: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    homes.sort();
    for home in homes {
        push_candidate(
            candidates,
            home.join("bin").join(java_binary_name()),
            JavaRuntimeSource::Managed,
        );
        push_candidate(
            candidates,
            home.join("Contents/Home/bin").join(java_binary_name()),
            JavaRuntimeSource::Managed,
        );
        discover_mojang_components(&home, candidates);
    }
}

fn discover_mojang_components(
    platform_root: &std::path::Path,
    candidates: &mut Vec<JavaCandidate>,
) {
    let Ok(entries) = std::fs::read_dir(platform_root) else {
        return;
    };
    let mut components: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    components.sort();
    for component in components {
        push_candidate(
            candidates,
            component.join("bin").join(java_binary_name()),
            JavaRuntimeSource::Managed,
        );
        push_candidate(
            candidates,
            component
                .join("jre.bundle/Contents/Home/bin")
                .join(java_binary_name()),
            JavaRuntimeSource::Managed,
        );
    }
}

fn push_candidate(
    candidates: &mut Vec<JavaCandidate>,
    executable: PathBuf,
    source: JavaRuntimeSource,
) {
    if !std::fs::metadata(&executable)
        .map(|metadata| metadata.is_file())
        .unwrap_or(false)
    {
        return;
    }
    let executable = std::fs::canonicalize(&executable).unwrap_or(executable);
    candidates.push(JavaCandidate { executable, source });
}

#[cfg(windows)]
fn java_binary_name() -> &'static str {
    "java.exe"
}

#[cfg(not(windows))]
fn java_binary_name() -> &'static str {
    "java"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_managed_layouts_without_touching_system_java() {
        let temporary = tempfile::TempDir::new().unwrap();
        let root = temporary.path();
        let java = java_binary_name();
        for relative in [
            format!("a/bin/{java}"),
            format!("b/Contents/Home/bin/{java}"),
        ] {
            let path = root.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"java").unwrap();
        }
        let detector = JavaDetector::new(vec![root.to_path_buf()])
            .with_java_home(false)
            .with_path(false);

        let candidates = detector.candidates();

        assert_eq!(candidates.len(), 2);
        assert!(
            candidates
                .iter()
                .all(|candidate| candidate.source == JavaRuntimeSource::Managed)
        );
        assert!(
            candidates
                .iter()
                .any(|candidate| candidate.executable.ends_with(format!("a/bin/{java}")))
        );
        assert!(candidates.iter().any(|candidate| {
            candidate
                .executable
                .ends_with(format!("Contents/Home/bin/{java}"))
        }));
    }

    #[test]
    fn discovers_mojang_store_component_layouts() {
        let temporary = tempfile::TempDir::new().unwrap();
        let root = temporary.path();
        let java = java_binary_name();
        for relative in [
            format!("linux/java-runtime-delta/bin/{java}"),
            format!("mac-os-arm64/java-runtime-delta/jre.bundle/Contents/Home/bin/{java}"),
        ] {
            let path = root.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"java").unwrap();
        }
        let detector = JavaDetector::new(vec![root.to_path_buf()])
            .with_java_home(false)
            .with_path(false);

        let candidates = detector.candidates();

        assert_eq!(candidates.len(), 2);
        assert!(candidates.iter().any(|candidate| {
            candidate
                .executable
                .ends_with(format!("linux/java-runtime-delta/bin/{java}"))
        }));
        assert!(candidates.iter().any(|candidate| {
            candidate.executable.ends_with(format!(
                "mac-os-arm64/java-runtime-delta/jre.bundle/Contents/Home/bin/{java}"
            ))
        }));
    }

    #[test]
    fn disables_paths_only_for_automatic_discovery() {
        let temporary = tempfile::TempDir::new().unwrap();
        let executable = temporary.path().join("a/bin").join(java_binary_name());
        std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
        std::fs::write(&executable, b"java").unwrap();
        let detector = JavaDetector::new(vec![temporary.path().to_path_buf()])
            .with_java_home(false)
            .with_path(false)
            .with_disabled_paths([executable]);

        assert!(detector.candidates().is_empty());
    }

    #[test]
    fn ignores_missing_managed_roots() {
        let detector = JavaDetector::new(vec![PathBuf::from("missing-managed-root")])
            .with_java_home(false)
            .with_path(false);
        assert!(detector.candidates().is_empty());
    }
}
