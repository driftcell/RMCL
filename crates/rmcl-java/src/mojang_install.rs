use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use rmcl_domain::{DownloadRequest, Sha1Hash};
use rmcl_instance::RepositoryPath;
use rmcl_net::{DownloadConfig, DownloadError, DownloadExecutor, MirrorHealth};
use serde::de::DeserializeOwned;
use sha1::{Digest, Sha1};
use tempfile::Builder;
use tokio::fs;
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::{
    JavaError, JavaRequirement, JavaRuntime, JavaRuntimeSource, MOJANG_RUNTIME_INDEX_URL,
    MarkerFile, MojangDownload, MojangFileType, MojangRuntimeEntry, MojangRuntimeIndex,
    MojangRuntimeManifest, RuntimeMarker, SelectedRuntime, mojang_platform_key, probe,
    runtime_components,
    runtime_path::{NormalizedLink, PathRegistry, normalize_link_target},
    select_runtime,
};

const MAX_MANIFEST_SIZE: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UrlPolicy {
    OfficialOnly,
    AnyHttp,
}

pub struct MojangRuntimeInstaller {
    store_root: PathBuf,
    downloader: DownloadExecutor,
    http: reqwest::Client,
    index_url: String,
    url_policy: UrlPolicy,
    mirror: Option<Arc<MirrorHealth>>,
}

impl MojangRuntimeInstaller {
    pub fn new(store_root: impl Into<PathBuf>) -> Result<Self, JavaError> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("RMCL/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .map_err(|source| JavaError::Http {
                url: "<client>".to_owned(),
                source,
            })?;
        Ok(Self {
            store_root: store_root.into(),
            downloader: DownloadExecutor::new(DownloadConfig::default())?,
            http,
            index_url: MOJANG_RUNTIME_INDEX_URL.to_owned(),
            url_policy: UrlPolicy::OfficialOnly,
            mirror: None,
        })
    }

    pub fn with_index_url(mut self, url: impl Into<String>) -> Self {
        self.index_url = url.into();
        self
    }

    pub fn with_url_policy(mut self, policy: UrlPolicy) -> Self {
        self.url_policy = policy;
        self
    }

    pub fn with_mirror(mut self, mirror: Arc<MirrorHealth>) -> Self {
        self.mirror = Some(mirror);
        self
    }

    fn sources_for(&self, official: &Url) -> Vec<Url> {
        self.mirror
            .as_ref()
            .map(|mirror| mirror.sources_for(official))
            .unwrap_or_else(|| vec![official.clone()])
    }

    pub async fn install(
        &self,
        requirement: &JavaRequirement,
        cancellation: CancellationToken,
    ) -> Result<JavaRuntime, JavaError> {
        let major = requirement.major_version.ok_or(JavaError::MissingMajor)?;
        let platform =
            mojang_platform_key(requirement.operating_system, &requirement.architecture)?;
        let components = runtime_components(requirement, major)?;
        let _lock = RuntimeInstallLock::acquire(&self.store_root, platform, &components.join("-"))?;

        for component in &components {
            if let Some(runtime) = self.try_reuse(requirement, component).await? {
                return Ok(runtime);
            }
        }

        let index = self.fetch_index().await?;
        let selected = select_runtime(&index, requirement)?;
        self.install_entry(requirement, &selected, cancellation)
            .await
    }

    async fn try_reuse(
        &self,
        requirement: &JavaRequirement,
        component: &str,
    ) -> Result<Option<JavaRuntime>, JavaError> {
        let platform =
            mojang_platform_key(requirement.operating_system, &requirement.architecture)?;
        let runtime_dir = self.runtime_dir(platform, component);
        let marker_path = self.marker_path(platform, component);
        let marker_type = path_type(&marker_path).await?;
        if marker_type.is_none() {
            if path_type(&runtime_dir).await?.is_some() {
                return Err(invalid_runtime(
                    &runtime_dir,
                    "runtime directory exists without a marker",
                ));
            }
            return Ok(None);
        }
        if marker_type != Some(PathType::File)
            || path_type(&runtime_dir).await? != Some(PathType::Directory)
        {
            return Err(invalid_runtime(
                &runtime_dir,
                "marker or runtime directory has an invalid type",
            ));
        }

        let marker = RuntimeMarker::load(&marker_path)
            .await
            .map_err(|error| invalid_runtime(&runtime_dir, error.to_string()))?;
        marker
            .validate(&runtime_dir, requirement, component)
            .await
            .map_err(|error| invalid_runtime(&runtime_dir, error.to_string()))?;
        let executable = runtime_dir.join(&marker.executable);
        let runtime = probe(executable, JavaRuntimeSource::Managed)
            .await
            .map_err(|error| invalid_runtime(&runtime_dir, error.to_string()))?;
        if !requirement.accepts(&runtime) {
            return Err(invalid_runtime(
                &runtime_dir,
                "probe does not satisfy the requirement",
            ));
        }
        if runtime.version != marker.runtime_version {
            return Err(invalid_runtime(
                &runtime_dir,
                "probe version does not match marker",
            ));
        }
        Ok(Some(runtime))
    }

    async fn install_entry(
        &self,
        requirement: &JavaRequirement,
        entry: &SelectedRuntime,
        cancellation: CancellationToken,
    ) -> Result<JavaRuntime, JavaError> {
        let platform =
            mojang_platform_key(requirement.operating_system, &requirement.architecture)?;
        let component = entry.component.clone();
        let manifest = self.fetch_manifest(&entry.entry).await?;
        let platform_dir = self.store_root.join(platform);
        fs::create_dir_all(&platform_dir)
            .await
            .map_err(|source| io_error(&platform_dir, source))?;
        let staging = Builder::new()
            .prefix(".staging-")
            .tempdir_in(&platform_dir)
            .map_err(|source| io_error(&platform_dir, source))?;
        let staging_root = staging.path().join("runtime");
        fs::create_dir(&staging_root)
            .await
            .map_err(|source| io_error(&staging_root, source))?;
        let runtime_dir = self.runtime_dir(platform, &component);
        let marker_path = self.marker_path(platform, &component);

        let mut registry = PathRegistry::default();
        let mut requests = Vec::new();
        let mut directories = Vec::new();
        let mut links = Vec::new();
        for (name, file) in &manifest.files {
            let path = RepositoryPath::new(name)?;
            registry.insert(&path, file.kind)?;
            match file.kind {
                MojangFileType::Directory => directories.push(path),
                MojangFileType::File => {
                    let raw = file
                        .downloads
                        .as_ref()
                        .and_then(|downloads| downloads.raw.as_ref())
                        .ok_or_else(|| JavaError::MissingRawDownload(name.clone()))?;
                    requests.push(self.raw_request(&path, raw, &staging_root)?);
                }
                MojangFileType::Link => {
                    let target =
                        file.target
                            .as_deref()
                            .ok_or_else(|| JavaError::InvalidManifest {
                                path: name.clone(),
                                reason: "link target is missing".to_owned(),
                            })?;
                    links.push(normalize_link_target(&path, target)?);
                }
            }
        }

        for directory in &directories {
            let path = directory.to_path(&staging_root);
            fs::create_dir_all(&path)
                .await
                .map_err(|source| io_error(&path, source))?;
            set_mode(&path, 0o755).await?;
        }

        let report = self
            .downloader
            .download_all(requests, cancellation, None)
            .await;
        if report.cancelled {
            return Err(JavaError::Download(DownloadError::Cancelled));
        }
        if !report.failures.is_empty() {
            return Err(JavaError::DownloadFailures {
                failures: report
                    .failures
                    .into_iter()
                    .map(|failure| {
                        format!(
                            "{}: {}",
                            failure.request.destination.display(),
                            failure.error
                        )
                    })
                    .collect(),
            });
        }

        for link in &links {
            create_link(&staging_root, link).await?;
        }
        for (name, file) in &manifest.files {
            if file.kind == MojangFileType::File {
                let path = RepositoryPath::new(name)?.to_path(&staging_root);
                let mode = if file.executable.unwrap_or(false) {
                    0o755
                } else {
                    0o644
                };
                set_mode(&path, mode).await?;
            }
        }

        let executable = find_java_executable(&staging_root, requirement.operating_system).await?;
        let mut marker = self.make_marker(
            requirement,
            &entry.entry,
            &component,
            &manifest,
            executable.clone(),
            &links,
        )?;
        marker
            .validate(&staging_root, requirement, &component)
            .await?;
        let runtime = probe(
            executable.to_path(&staging_root),
            JavaRuntimeSource::Managed,
        )
        .await
        .map_err(|error| JavaError::ProbeRejected {
            path: executable.to_path(&staging_root).display().to_string(),
            reason: error.to_string(),
        })?;
        if !requirement.accepts(&runtime) {
            return Err(JavaError::ProbeRejected {
                path: executable.to_path(&staging_root).display().to_string(),
                reason: "probe does not satisfy the requirement".to_owned(),
            });
        }
        marker.runtime_version = runtime.version.clone();
        marker.vendor = runtime.vendor.clone();
        marker.installed_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0);

        if path_type(&runtime_dir).await?.is_some() {
            return Err(invalid_runtime(
                &runtime_dir,
                "refusing to replace an existing runtime",
            ));
        }
        fs::rename(&staging_root, &runtime_dir)
            .await
            .map_err(|source| JavaError::Publish {
                path: runtime_dir.display().to_string(),
                source,
            })?;
        drop(staging);
        marker.write(&marker_path).await?;
        Ok(JavaRuntime {
            executable: executable.to_path(&runtime_dir),
            ..runtime
        })
    }

    fn make_marker(
        &self,
        requirement: &JavaRequirement,
        entry: &MojangRuntimeEntry,
        component: &str,
        manifest: &MojangRuntimeManifest,
        executable: RepositoryPath,
        links: &[NormalizedLink],
    ) -> Result<RuntimeMarker, JavaError> {
        let files = manifest
            .files
            .iter()
            .map(|(name, file)| {
                let marker_file = match file.kind {
                    MojangFileType::File => {
                        let raw = file
                            .downloads
                            .as_ref()
                            .and_then(|downloads| downloads.raw.as_ref());
                        MarkerFile::File {
                            sha1: raw.and_then(|download| download.sha1.clone()),
                            size: raw.and_then(|download| download.size),
                            executable: file.executable.unwrap_or(false),
                        }
                    }
                    MojangFileType::Directory => MarkerFile::Directory,
                    MojangFileType::Link => links
                        .iter()
                        .find(|link| link.source.as_str() == name)
                        .map(|link| MarkerFile::Link {
                            target: link.target.as_str().to_owned(),
                        })
                        .ok_or_else(|| JavaError::InvalidManifest {
                            path: name.clone(),
                            reason: "link target was not normalized".to_owned(),
                        })?,
                };
                Ok((name.clone(), marker_file))
            })
            .collect::<Result<BTreeMap<_, _>, JavaError>>()?;
        Ok(RuntimeMarker {
            schema_version: 1,
            source: "mojang".to_owned(),
            platform: mojang_platform_key(requirement.operating_system, &requirement.architecture)?
                .to_owned(),
            component: component.to_owned(),
            requested_major: requirement.major_version.unwrap_or_default(),
            manifest_url: entry.manifest.url.clone(),
            manifest_sha1: entry.manifest.sha1.clone(),
            manifest_size: entry.manifest.size,
            executable: executable.as_str().to_owned(),
            runtime_version: String::new(),
            vendor: None,
            installed_at: 0,
            files,
        })
    }

    fn raw_request(
        &self,
        path: &RepositoryPath,
        raw: &MojangDownload,
        staging_root: &Path,
    ) -> Result<DownloadRequest, JavaError> {
        let url = self.validate_url(&raw.url)?;
        let sha1 = raw
            .sha1
            .as_deref()
            .ok_or_else(|| JavaError::MissingIntegrity(path.to_string()))?
            .parse()?;
        let size = raw
            .size
            .ok_or_else(|| JavaError::MissingIntegrity(path.to_string()))?;
        Ok(DownloadRequest {
            sources: self.sources_for(&url),
            destination: path.to_path(staging_root),
            sha1: Some(sha1),
            size: Some(size),
        })
    }

    async fn fetch_index(&self) -> Result<MojangRuntimeIndex, JavaError> {
        self.fetch_json(&self.index_url, None, None).await
    }

    async fn fetch_manifest(
        &self,
        entry: &MojangRuntimeEntry,
    ) -> Result<MojangRuntimeManifest, JavaError> {
        let sha1 = entry
            .manifest
            .sha1
            .as_deref()
            .ok_or_else(|| JavaError::MissingIntegrity(entry.manifest.url.clone()))?
            .parse()?;
        let size = entry
            .manifest
            .size
            .ok_or_else(|| JavaError::MissingIntegrity(entry.manifest.url.clone()))?;
        self.fetch_json(&entry.manifest.url, Some(sha1), Some(size))
            .await
    }

    async fn fetch_json<T: DeserializeOwned>(
        &self,
        url: &str,
        expected_sha1: Option<Sha1Hash>,
        expected_size: Option<u64>,
    ) -> Result<T, JavaError> {
        let official = self.validate_url(url)?;
        let sources = self.sources_for(&official);
        let mut last_error = None;
        for source in sources {
            match self
                .fetch_json_from(&source, expected_sha1, expected_size)
                .await
            {
                Ok(value) => return Ok(value),
                Err(error) => last_error = Some(error),
            }
        }
        Err(last_error.unwrap_or_else(|| JavaError::HttpStatus {
            url: official.to_string(),
            status: reqwest::StatusCode::SERVICE_UNAVAILABLE,
        }))
    }

    async fn fetch_json_from<T: DeserializeOwned>(
        &self,
        url: &Url,
        expected_sha1: Option<Sha1Hash>,
        expected_size: Option<u64>,
    ) -> Result<T, JavaError> {
        let response =
            self.http
                .get(url.clone())
                .send()
                .await
                .map_err(|source| JavaError::Http {
                    url: url.to_string(),
                    source,
                })?;
        if response
            .content_length()
            .is_some_and(|length| length > MAX_MANIFEST_SIZE as u64)
        {
            return Err(JavaError::ManifestTooLarge {
                limit: MAX_MANIFEST_SIZE as u64,
            });
        }
        let status = response.status();
        if !status.is_success() {
            return Err(JavaError::HttpStatus {
                url: url.to_string(),
                status,
            });
        }
        let bytes = response.bytes().await.map_err(|source| JavaError::Http {
            url: url.to_string(),
            source,
        })?;
        if bytes.len() > MAX_MANIFEST_SIZE {
            return Err(JavaError::ManifestTooLarge {
                limit: MAX_MANIFEST_SIZE as u64,
            });
        }
        if let Some(expected) = expected_size
            && bytes.len() as u64 != expected
        {
            return Err(JavaError::ManifestSizeMismatch {
                expected,
                actual: bytes.len() as u64,
            });
        }
        if let Some(expected) = expected_sha1 {
            let digest = Sha1::digest(&bytes);
            let mut raw = [0_u8; 20];
            raw.copy_from_slice(&digest);
            let actual = Sha1Hash::from_bytes(raw);
            if actual != expected {
                return Err(JavaError::ManifestHashMismatch { expected, actual });
            }
        }
        serde_json::from_slice(&bytes).map_err(|source| JavaError::ManifestJson {
            path: url.to_string(),
            source,
        })
    }

    fn validate_url(&self, value: &str) -> Result<Url, JavaError> {
        let url = Url::parse(value).map_err(|source| JavaError::InvalidUrl {
            url: value.to_owned(),
            source,
        })?;
        match self.url_policy {
            UrlPolicy::OfficialOnly => {
                if url.scheme() != "https"
                    || !matches!(
                        url.host_str(),
                        Some("piston-meta.mojang.com" | "piston-data.mojang.com")
                    )
                {
                    return Err(JavaError::UnexpectedHost(url.to_string()));
                }
            }
            UrlPolicy::AnyHttp => {
                if !matches!(url.scheme(), "http" | "https") {
                    return Err(JavaError::UnexpectedHost(url.to_string()));
                }
            }
        }
        Ok(url)
    }

    fn runtime_dir(&self, platform: &str, component: &str) -> PathBuf {
        self.store_root.join(platform).join(component)
    }

    fn marker_path(&self, platform: &str, component: &str) -> PathBuf {
        self.store_root
            .join(platform)
            .join(format!("{component}.json"))
    }
}

#[derive(Debug)]
pub struct RuntimeInstallLock {
    path: PathBuf,
}

impl RuntimeInstallLock {
    pub fn acquire(store_root: &Path, platform: &str, component: &str) -> Result<Self, JavaError> {
        let directory = store_root.join(".locks");
        std::fs::create_dir_all(&directory).map_err(|source| JavaError::Publish {
            path: directory.display().to_string(),
            source,
        })?;
        let path = directory.join(format!("{platform}-{component}.lock"));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(_) => Ok(Self { path }),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                Err(JavaError::AlreadyInstalling(path.display().to_string()))
            }
            Err(source) => Err(JavaError::Publish {
                path: path.display().to_string(),
                source,
            }),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for RuntimeInstallLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PathType {
    File,
    Directory,
}

async fn path_type(path: &Path) -> Result<Option<PathType>, JavaError> {
    match fs::symlink_metadata(path).await {
        Ok(metadata) => {
            let file_type = metadata.file_type();
            if file_type.is_file() {
                Ok(Some(PathType::File))
            } else if file_type.is_dir() {
                Ok(Some(PathType::Directory))
            } else {
                Ok(None)
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(io_error(path, source)),
    }
}

async fn find_java_executable(
    root: &Path,
    os: rmcl_domain::OperatingSystem,
) -> Result<RepositoryPath, JavaError> {
    let candidates: &[&str] = match os {
        rmcl_domain::OperatingSystem::MacOs => &["jre.bundle/Contents/Home/bin/java", "bin/java"],
        rmcl_domain::OperatingSystem::Windows => &["bin/java.exe", "bin/java"],
        rmcl_domain::OperatingSystem::Linux => &["bin/java"],
    };
    for candidate in candidates {
        let path = RepositoryPath::new(*candidate)?;
        if path_type(&path.to_path(root)).await? == Some(PathType::File) {
            return Ok(path);
        }
    }
    Err(JavaError::MissingJavaExecutable(root.display().to_string()))
}

async fn create_link(root: &Path, link: &NormalizedLink) -> Result<(), JavaError> {
    let target = link.target.to_path(root);
    let metadata = fs::symlink_metadata(&target)
        .await
        .map_err(|source| io_error(&target, source))?;
    if !metadata.is_file() && !metadata.is_dir() {
        return Err(JavaError::MissingLinkTarget(target.display().to_string()));
    }
    let source_path = link.source.to_path(root);
    if path_type(&source_path).await?.is_some() {
        return Err(JavaError::PathConflict {
            path: source_path.display().to_string(),
            existing: target.display().to_string(),
        });
    }
    if let Some(parent) = source_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .await
            .map_err(|source| io_error(parent, source))?;
    }

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(link.relative.as_str(), &source_path).map_err(|source| {
            JavaError::LinkCreation {
                path: source_path.display().to_string(),
                source,
            }
        })?;
    }
    #[cfg(windows)]
    {
        let result = if metadata.is_dir() {
            std::os::windows::fs::symlink_dir(link.relative.as_str(), &source_path)
        } else {
            std::os::windows::fs::symlink_file(link.relative.as_str(), &source_path)
        };
        result.map_err(|source| JavaError::LinkCreation {
            path: source_path.display().to_string(),
            source,
        })?;
    }
    Ok(())
}

async fn set_mode(path: &Path, mode: u32) -> Result<(), JavaError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
            .await
            .map_err(|source| io_error(path, source))?;
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
    }
    Ok(())
}

fn invalid_runtime(path: &Path, reason: impl Into<String>) -> JavaError {
    JavaError::ExistingRuntimeInvalid {
        path: path.display().to_string(),
        reason: reason.into(),
    }
}

fn io_error(path: &Path, source: std::io::Error) -> JavaError {
    JavaError::Io {
        path: path.display().to_string(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcl_domain::OperatingSystem;
    use rmcl_net::MirrorPreference;

    #[tokio::test]
    async fn finds_windows_java_executable() {
        let temporary = tempfile::tempdir().unwrap();
        let bin = temporary.path().join("bin");
        tokio::fs::create_dir_all(&bin).await.unwrap();
        tokio::fs::write(bin.join("java.exe"), b"fake")
            .await
            .unwrap();

        let executable = find_java_executable(temporary.path(), OperatingSystem::Windows)
            .await
            .unwrap();

        assert_eq!(executable.as_str(), "bin/java.exe");
    }

    #[test]
    fn mirror_adds_a_candidate_before_official_runtime_files() {
        let temporary = tempfile::tempdir().unwrap();
        let installer = MojangRuntimeInstaller::new(temporary.path())
            .unwrap()
            .with_mirror(Arc::new(MirrorHealth::new(MirrorPreference::Mirror)));
        let official = Url::parse("https://piston-data.mojang.com/v1/objects/abc/file").unwrap();

        let sources = installer.sources_for(&official);

        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].host_str(), Some("bmclapi2.bangbang93.com"));
        assert_eq!(sources[1], official);
    }
}
