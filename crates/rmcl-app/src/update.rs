//! GitHub Release update checks, verified package downloads, and staged install.

use std::{
    env,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use flate2::read::GzDecoder;
use reqwest::{Client, header};
use rmcl_archive::{ArchiveExtractor, ExtractionLimits};
use rmcl_domain::ExtractionPlan;
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use url::Url;

const DEFAULT_API_URL: &str = "https://api.github.com/repos/driftcell/RMCL/releases/latest";
const UPDATE_USER_AGENT: &str = concat!(
    "RMCL/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/driftcell/RMCL)"
);
const CHECKSUMS_ASSET: &str = "SHA256SUMS";
const PARENT_WAIT: Duration = Duration::from_secs(30);
const PARENT_POLL: Duration = Duration::from_millis(200);
const ZIP_LIMITS: ExtractionLimits = ExtractionLimits {
    max_entries: 10_000,
    max_entry_size: 256 * 1024 * 1024,
    max_total_size: 1024 * 1024 * 1024,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UpdatePackage {
    MacOsAppZip,
    WindowsZip,
    LinuxTarGz,
}

impl UpdatePackage {
    pub const fn label(self) -> &'static str {
        match self {
            Self::MacOsAppZip => "macOS .app",
            Self::WindowsZip => "Windows ZIP",
            Self::LinuxTarGz => "Linux tar.gz",
        }
    }
}

#[derive(Debug, Clone)]
pub struct UpdateAsset {
    pub name: String,
    pub url: Url,
    pub size: Option<u64>,
    pub package: UpdatePackage,
}

#[derive(Debug, Clone)]
pub struct UpdateRelease {
    pub version: Version,
    pub tag: String,
    pub title: String,
    pub notes: Option<String>,
    pub page_url: Url,
    pub asset: UpdateAsset,
    pub sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpdateProgress {
    pub downloaded: u64,
    pub total: Option<u64>,
}

#[derive(Clone)]
pub struct UpdateClient {
    client: Client,
    api_url: Url,
}

impl UpdateClient {
    pub fn new(api_url: Url) -> Result<Self, UpdateError> {
        let client = Client::builder()
            .user_agent(UPDATE_USER_AGENT)
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(UpdateError::Http)?;
        Ok(Self { client, api_url })
    }

    pub fn github() -> Result<Self, UpdateError> {
        let url = match env::var("RMCL_UPDATE_API") {
            Ok(value) => {
                Url::parse(&value).map_err(|error| UpdateError::Configuration(error.to_string()))?
            }
            Err(_) => Url::parse(DEFAULT_API_URL)
                .map_err(|error| UpdateError::Configuration(error.to_string()))?,
        };
        Self::new(url)
    }

    pub fn api_url(&self) -> &Url {
        &self.api_url
    }

    /// Checks the configured release endpoint for a newer stable release.
    ///
    /// Automatic checks should treat errors as non-fatal and retry later.
    pub async fn check(
        &self,
        current_version: &str,
        cancellation: &CancellationToken,
    ) -> Result<Option<UpdateRelease>, UpdateError> {
        let current = parse_version(current_version)?;
        let response = self
            .get(self.api_url.clone(), cancellation)
            .await?
            .error_for_status()
            .map_err(UpdateError::Http)?;
        let release: ReleaseResponse = response.json().await.map_err(UpdateError::Http)?;
        if release.draft || release.prerelease {
            return Ok(None);
        }
        let tag_version = release.tag_name.trim_start_matches('v');
        let version = parse_version(tag_version)?;
        if version <= current {
            return Ok(None);
        }
        let Some(asset) = select_asset(&release.assets, env::consts::OS, env::consts::ARCH)? else {
            return Err(UpdateError::NoAsset {
                os: env::consts::OS.to_owned(),
                arch: env::consts::ARCH.to_owned(),
            });
        };
        let checksum_asset = release
            .assets
            .iter()
            .find(|asset| asset.name == CHECKSUMS_ASSET)
            .ok_or(UpdateError::MissingChecksums)?;
        let checksums = self
            .get(
                Url::parse(&checksum_asset.browser_download_url)
                    .map_err(|error| UpdateError::Configuration(error.to_string()))?,
                cancellation,
            )
            .await?
            .error_for_status()
            .map_err(UpdateError::Http)?
            .text()
            .await
            .map_err(UpdateError::Http)?;
        let sha256 = checksum_for(&checksums, &asset.name)
            .ok_or_else(|| UpdateError::MissingChecksum(asset.name.clone()))?;
        Ok(Some(UpdateRelease {
            version,
            tag: release.tag_name,
            title: release.name.unwrap_or_else(|| "RMCL".to_owned()),
            notes: release.body,
            page_url: Url::parse(&release.html_url)
                .map_err(|error| UpdateError::Configuration(error.to_string()))?,
            asset: UpdateAsset {
                name: asset.name.clone(),
                url: Url::parse(&asset.browser_download_url)
                    .map_err(|error| UpdateError::Configuration(error.to_string()))?,
                size: asset.size,
                package: package_for_asset(&asset.name)
                    .ok_or_else(|| UpdateError::UnsupportedPackage(asset.name.clone()))?,
            },
            sha256,
        }))
    }

    /// Downloads and verifies one update package, reusing a verified cache file.
    pub async fn download(
        &self,
        release: &UpdateRelease,
        directory: &Path,
        progress: Option<mpsc::UnboundedSender<UpdateProgress>>,
        cancellation: &CancellationToken,
    ) -> Result<PathBuf, UpdateError> {
        fs::create_dir_all(directory).map_err(|source| UpdateError::Io {
            operation: "create update directory",
            path: directory.to_path_buf(),
            source,
        })?;
        let filename = Path::new(&release.asset.name).file_name().ok_or_else(|| {
            UpdateError::Configuration("update asset has no file name".to_owned())
        })?;
        let destination = directory.join(filename);
        if destination.is_file() && verify_sha256(&destination, &release.sha256).is_ok() {
            if let Some(progress) = progress {
                let _ = progress.send(UpdateProgress {
                    downloaded: fs::metadata(&destination).map_or(0, |meta| meta.len()),
                    total: Some(release.asset.size.unwrap_or_default()),
                });
            }
            return Ok(destination);
        }

        let temporary = directory.join(format!(
            ".rmcl-update-{}-{}.tmp",
            std::process::id(),
            release.sha256.get(..12).unwrap_or(&release.sha256)
        ));
        let result = async {
            let response = self
                .get(release.asset.url.clone(), cancellation)
                .await?
                .error_for_status()
                .map_err(UpdateError::Http)?;
            let total = response.content_length().or(release.asset.size);
            let mut file = tokio::fs::File::create(&temporary)
                .await
                .map_err(|source| UpdateError::Io {
                    operation: "create update temporary file",
                    path: temporary.clone(),
                    source,
                })?;
            let mut hasher = Sha256::new();
            let mut downloaded = 0u64;
            let mut last_percent = None;
            let mut response = response;
            loop {
                let chunk = tokio::select! {
                    _ = cancellation.cancelled() => {
                        return Err(UpdateError::Cancelled);
                    }
                    chunk = response.chunk() => chunk.map_err(UpdateError::Http)?,
                };
                let Some(chunk) = chunk else {
                    break;
                };
                use tokio::io::AsyncWriteExt as _;
                file.write_all(&chunk)
                    .await
                    .map_err(|source| UpdateError::Io {
                        operation: "write update package",
                        path: temporary.clone(),
                        source,
                    })?;
                hasher.update(&chunk);
                downloaded += chunk.len() as u64;
                if let Some(progress) = &progress {
                    let percent =
                        total.map(|total| (downloaded.saturating_mul(100) / total.max(1)).min(100));
                    if percent != last_percent {
                        last_percent = percent;
                        let _ = progress.send(UpdateProgress { downloaded, total });
                    }
                }
            }
            use tokio::io::AsyncWriteExt as _;
            file.flush().await.map_err(|source| UpdateError::Io {
                operation: "flush update package",
                path: temporary.clone(),
                source,
            })?;
            let actual = format!("{:x}", hasher.finalize());
            if !actual.eq_ignore_ascii_case(&release.sha256) {
                return Err(UpdateError::ChecksumMismatch {
                    expected: release.sha256.clone(),
                    actual,
                });
            }
            if destination.exists() {
                tokio::fs::remove_file(&destination)
                    .await
                    .map_err(|source| UpdateError::Io {
                        operation: "replace cached update package",
                        path: destination.clone(),
                        source,
                    })?;
            }
            tokio::fs::rename(&temporary, &destination)
                .await
                .map_err(|source| UpdateError::Io {
                    operation: "publish update package",
                    path: destination.clone(),
                    source,
                })?;
            Ok(destination)
        }
        .await;
        if result.is_err() {
            let _ = tokio::fs::remove_file(&temporary).await;
        }
        result
    }

    async fn get(
        &self,
        url: Url,
        cancellation: &CancellationToken,
    ) -> Result<reqwest::Response, UpdateError> {
        tokio::select! {
            _ = cancellation.cancelled() => Err(UpdateError::Cancelled),
            response = self.client.get(url)
                .header(header::ACCEPT, "application/vnd.github+json")
                .send() => response.map_err(UpdateError::Http),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ReleaseResponse {
    tag_name: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    body: Option<String>,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: Option<u64>,
}

fn parse_version(value: &str) -> Result<Version, UpdateError> {
    Version::parse(value.trim().trim_start_matches('v'))
        .map_err(|error| UpdateError::InvalidVersion(error.to_string()))
}

fn select_asset<'a>(
    assets: &'a [ReleaseAsset],
    os: &str,
    arch: &str,
) -> Result<Option<&'a ReleaseAsset>, UpdateError> {
    let suffix = match (os, arch) {
        ("macos", "aarch64") => "-macos-aarch64.zip",
        ("macos", "x86_64") => "-macos-x86_64.zip",
        ("windows", "x86_64") => "-windows-x86_64.zip",
        ("linux", "x86_64") => "-linux-x86_64.tar.gz",
        _ => {
            return Err(UpdateError::NoAsset {
                os: os.to_owned(),
                arch: arch.to_owned(),
            });
        }
    };
    Ok(assets.iter().find(|asset| asset.name.ends_with(suffix)))
}

fn package_for_asset(name: &str) -> Option<UpdatePackage> {
    if name.ends_with("-macos-aarch64.zip") || name.ends_with("-macos-x86_64.zip") {
        Some(UpdatePackage::MacOsAppZip)
    } else if name.ends_with("-windows-x86_64.zip") {
        Some(UpdatePackage::WindowsZip)
    } else if name.ends_with("-linux-x86_64.tar.gz") {
        Some(UpdatePackage::LinuxTarGz)
    } else {
        None
    }
}

fn checksum_for(checksums: &str, filename: &str) -> Option<String> {
    checksums.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let name = parts.next()?;
        let name = name.trim_start_matches('*');
        (name == filename && hash.len() == 64).then(|| hash.to_ascii_lowercase())
    })
}

fn verify_sha256(path: &Path, expected: &str) -> Result<(), UpdateError> {
    let mut file = File::open(path).map_err(|source| UpdateError::Io {
        operation: "open downloaded update",
        path: path.to_path_buf(),
        source,
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|source| UpdateError::Io {
            operation: "read downloaded update",
            path: path.to_path_buf(),
            source,
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let actual = format!("{:x}", hasher.finalize());
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(UpdateError::ChecksumMismatch {
            expected: expected.to_owned(),
            actual,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdatePlan {
    pub archive: PathBuf,
    pub target: PathBuf,
    pub package: UpdatePackage,
    pub sha256: String,
    pub parent_pid: u32,
    pub restart_args: Vec<String>,
    pub log: PathBuf,
}

impl UpdatePlan {
    pub fn prepare(
        release: &UpdateRelease,
        archive: &Path,
        log_directory: &Path,
    ) -> Result<Self, UpdateError> {
        let target = install_target()?;
        fs::create_dir_all(log_directory).map_err(|source| UpdateError::Io {
            operation: "create update log directory",
            path: log_directory.to_path_buf(),
            source,
        })?;
        let mut restart_args = env::args_os()
            .skip(1)
            .filter_map(|argument| argument.into_string().ok())
            .collect::<Vec<_>>();
        restart_args.retain(|argument| argument != "--apply-update");
        Ok(Self {
            archive: archive.to_path_buf(),
            target,
            package: release.asset.package,
            sha256: release.sha256.clone(),
            parent_pid: std::process::id(),
            restart_args,
            log: log_directory.join(format!("update-{}.log", std::process::id())),
        })
    }
}

fn install_target() -> Result<PathBuf, UpdateError> {
    let executable = env::current_exe().map_err(|source| UpdateError::Io {
        operation: "resolve current executable",
        path: PathBuf::from("<current executable>"),
        source,
    })?;
    if cfg!(target_os = "macos") {
        let app = executable
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .filter(|path| path.extension().is_some_and(|extension| extension == "app"))
            .ok_or_else(|| {
                UpdateError::UnsupportedInstall(
                    "macOS update requires running from an RMCL.app bundle".to_owned(),
                )
            })?;
        return Ok(app.to_path_buf());
    }
    Ok(executable)
}

/// Spawns the same executable from a temporary path so the running app can quit
/// before its own files are replaced.
pub fn spawn_apply_helper(plan: &UpdatePlan) -> Result<(), UpdateError> {
    let plan_path = plan.log.with_extension("json");
    let json = serde_json::to_vec_pretty(plan)
        .map_err(|error| UpdateError::Configuration(error.to_string()))?;
    fs::write(&plan_path, json).map_err(|source| UpdateError::Io {
        operation: "write update plan",
        path: plan_path.clone(),
        source,
    })?;

    let current = env::current_exe().map_err(|source| UpdateError::Io {
        operation: "resolve current executable",
        path: PathBuf::from("<current executable>"),
        source,
    })?;
    let helper = plan
        .log
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!(
            "rmcl-updater-{}{}",
            std::process::id(),
            std::env::consts::EXE_SUFFIX
        ));
    fs::copy(&current, &helper).map_err(|source| UpdateError::Io {
        operation: "copy update helper",
        path: helper.clone(),
        source,
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mut permissions = fs::metadata(&helper)
            .map_err(|source| UpdateError::Io {
                operation: "inspect update helper",
                path: helper.clone(),
                source,
            })?
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&helper, permissions).map_err(|source| UpdateError::Io {
            operation: "make update helper executable",
            path: helper.clone(),
            source,
        })?;
    }

    let mut command = Command::new(&helper);
    command
        .arg("--apply-update")
        .arg(&plan_path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        command.creation_flags(0x0800_0000);
    }
    command.spawn().map_err(|source| UpdateError::Io {
        operation: "spawn update helper",
        path: helper,
        source,
    })?;
    Ok(())
}

/// Handles the helper-only `--apply-update <plan>` mode before GUI startup.
pub fn apply_update_from_args() -> Option<Result<(), UpdateError>> {
    let mut arguments = env::args_os().skip(1);
    if arguments.next().as_deref() != Some(std::ffi::OsStr::new("--apply-update")) {
        return None;
    }
    let path = arguments.next()?;
    Some(apply_update(Path::new(&path)))
}

fn apply_update(plan_path: &Path) -> Result<(), UpdateError> {
    let plan: UpdatePlan =
        serde_json::from_slice(&fs::read(plan_path).map_err(|source| UpdateError::Io {
            operation: "read update plan",
            path: plan_path.to_path_buf(),
            source,
        })?)
        .map_err(|error| UpdateError::Configuration(error.to_string()))?;
    let result = apply_update_inner(&plan);
    match &result {
        Ok(()) => append_log(&plan.log, "update completed"),
        Err(error) => {
            append_log(&plan.log, &format!("update failed: {error}"));
            if !matches!(error, UpdateError::ParentStillRunning) {
                let _ = launch_target(&plan.target, &plan.restart_args);
            }
        }
    }
    result
}

fn apply_update_inner(plan: &UpdatePlan) -> Result<(), UpdateError> {
    wait_for_parent(plan.parent_pid)?;
    verify_sha256(&plan.archive, &plan.sha256)?;
    let target_parent = plan.target.parent().ok_or_else(|| {
        UpdateError::UnsupportedInstall("update target has no parent directory".to_owned())
    })?;
    fs::create_dir_all(target_parent).map_err(|source| UpdateError::Io {
        operation: "create update target directory",
        path: target_parent.to_path_buf(),
        source,
    })?;
    let staging = tempfile::Builder::new()
        .prefix(".rmcl-update-")
        .tempdir_in(target_parent)
        .map_err(|source| UpdateError::Io {
            operation: "create update staging directory",
            path: target_parent.to_path_buf(),
            source,
        })?;
    extract_package(&plan.archive, plan.package, staging.path())?;

    match plan.package {
        UpdatePackage::MacOsAppZip => {
            let extracted = staging.path().join("extracted");
            let new_app = extracted.join("RMCL.app");
            if !new_app.is_dir() {
                return Err(UpdateError::UnsupportedInstall(
                    "macOS update archive does not contain RMCL.app".to_owned(),
                ));
            }
            replace_path(&new_app, &plan.target)?;
            launch_target(&plan.target, &plan.restart_args)?;
        }
        UpdatePackage::WindowsZip => {
            let extracted = staging.path().join("extracted");
            let new_exe = find_file(&extracted, "rmcl.exe").ok_or_else(|| {
                UpdateError::UnsupportedInstall(
                    "Windows update archive does not contain rmcl.exe".to_owned(),
                )
            })?;
            replace_file(&new_exe, &plan.target)?;
            launch_target(&plan.target, &plan.restart_args)?;
        }
        UpdatePackage::LinuxTarGz => {
            let new_exe = find_file(staging.path(), "rmcl").ok_or_else(|| {
                UpdateError::UnsupportedInstall(
                    "Linux update archive does not contain rmcl".to_owned(),
                )
            })?;
            replace_file(&new_exe, &plan.target)?;
            launch_target(&plan.target, &plan.restart_args)?;
        }
    }
    append_log(&plan.log, "update applied and restarted");
    Ok(())
}

fn extract_package(
    archive: &Path,
    package: UpdatePackage,
    destination: &Path,
) -> Result<(), UpdateError> {
    match package {
        UpdatePackage::MacOsAppZip | UpdatePackage::WindowsZip => {
            let extracted = destination.join("extracted");
            ArchiveExtractor::new(ZIP_LIMITS)
                .extract_zip(
                    &ExtractionPlan {
                        source: archive.to_path_buf(),
                        destination: extracted,
                        exclude_prefixes: Vec::new(),
                    },
                    CancellationToken::new(),
                    None,
                )
                .map_err(|error| UpdateError::Extraction(error.to_string()))?;
        }
        UpdatePackage::LinuxTarGz => {
            let file = File::open(archive).map_err(|source| UpdateError::Io {
                operation: "open Linux update archive",
                path: archive.to_path_buf(),
                source,
            })?;
            let mut archive = tar::Archive::new(GzDecoder::new(file));
            archive.set_preserve_permissions(true);
            archive.set_overwrite(true);
            archive
                .unpack(destination)
                .map_err(|source| UpdateError::Io {
                    operation: "extract Linux update archive",
                    path: destination.to_path_buf(),
                    source,
                })?;
        }
    }
    Ok(())
}

fn find_file(directory: &Path, name: &str) -> Option<PathBuf> {
    let entries = fs::read_dir(directory).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_file(&path, name) {
                return Some(found);
            }
        } else if path.file_name().and_then(|file| file.to_str()) == Some(name) {
            return Some(path);
        }
    }
    None
}

fn replace_path(new: &Path, target: &Path) -> Result<(), UpdateError> {
    let backup = target.with_extension("old");
    remove_path(&backup);
    if target.exists() {
        fs::rename(target, &backup).map_err(|source| UpdateError::Io {
            operation: "move old application aside",
            path: target.to_path_buf(),
            source,
        })?;
    }
    if let Err(error) = fs::rename(new, target) {
        let _ = fs::rename(&backup, target);
        return Err(UpdateError::Io {
            operation: "publish new application",
            path: target.to_path_buf(),
            source: error,
        });
    }
    remove_path(&backup);
    Ok(())
}

fn replace_file(new: &Path, target: &Path) -> Result<(), UpdateError> {
    let backup = target.with_extension("old");
    remove_path(&backup);
    if target.exists() {
        fs::rename(target, &backup).map_err(|source| UpdateError::Io {
            operation: "move old executable aside",
            path: target.to_path_buf(),
            source,
        })?;
    }
    if let Err(error) = fs::rename(new, target) {
        let _ = fs::rename(&backup, target);
        return Err(UpdateError::Io {
            operation: "publish new executable",
            path: target.to_path_buf(),
            source: error,
        });
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = fs::set_permissions(target, fs::Permissions::from_mode(0o755));
    }
    remove_path(&backup);
    Ok(())
}

fn remove_path(path: &Path) {
    if path.is_dir() {
        let _ = fs::remove_dir_all(path);
    } else {
        let _ = fs::remove_file(path);
    }
}

fn wait_for_parent(pid: u32) -> Result<(), UpdateError> {
    let pid = sysinfo::Pid::from_u32(pid);
    let mut system = sysinfo::System::new();
    let started = std::time::Instant::now();
    loop {
        system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
        if system.process(pid).is_none() {
            return Ok(());
        }
        if started.elapsed() > PARENT_WAIT {
            return Err(UpdateError::ParentStillRunning);
        }
        std::thread::sleep(PARENT_POLL);
    }
}

fn launch_target(target: &Path, arguments: &[String]) -> Result<(), UpdateError> {
    #[cfg(target_os = "macos")]
    {
        let mut command = Command::new("/usr/bin/open");
        command.arg("-n").arg(target);
        if !arguments.is_empty() {
            command.arg("--args").args(arguments);
        }
        command.spawn().map_err(|source| UpdateError::Io {
            operation: "restart updated application",
            path: target.to_path_buf(),
            source,
        })?;
    }
    #[cfg(not(target_os = "macos"))]
    {
        Command::new(target)
            .args(arguments)
            .spawn()
            .map_err(|source| UpdateError::Io {
                operation: "restart updated application",
                path: target.to_path_buf(),
                source,
            })?;
    }
    Ok(())
}

fn append_log(path: &Path, message: &str) {
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{message}");
    }
}

#[derive(Debug, Error)]
pub enum UpdateError {
    #[error("update HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("update configuration error: {0}")]
    Configuration(String),
    #[error("invalid release version: {0}")]
    InvalidVersion(String),
    #[error("no update asset for {os}/{arch}")]
    NoAsset { os: String, arch: String },
    #[error("release has no SHA256SUMS asset")]
    MissingChecksums,
    #[error("release checksum is missing for `{0}`")]
    MissingChecksum(String),
    #[error("unsupported update package `{0}`")]
    UnsupportedPackage(String),
    #[error("update checksum mismatch: expected {expected}, actual {actual}")]
    ChecksumMismatch { expected: String, actual: String },
    #[error("update was cancelled")]
    Cancelled,
    #[error("unsupported update installation: {0}")]
    UnsupportedInstall(String),
    #[error("the running application did not exit before the update timeout")]
    ParentStillRunning,
    #[error("update archive extraction failed: {0}")]
    Extraction(String),
    #[error("failed to {operation} `{path}`: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[cfg(test)]
mod tests {
    use axum::{Json, Router, routing::get};
    use serde_json::json;

    use super::*;

    fn current_asset_name(version: &str) -> String {
        let suffix = match (env::consts::OS, env::consts::ARCH) {
            ("macos", "aarch64") => "macos-aarch64.zip",
            ("macos", "x86_64") => "macos-x86_64.zip",
            ("windows", "x86_64") => "windows-x86_64.zip",
            _ => "linux-x86_64.tar.gz",
        };
        format!("RMCL-{version}-{suffix}")
    }

    fn asset_json(base: &Url, name: &str) -> serde_json::Value {
        json!({
            "name": name,
            "browser_download_url": base.join(&format!("assets/{name}")).unwrap().to_string(),
            "size": 42
        })
    }

    async fn serve(
        listener: tokio::net::TcpListener,
        router: Router,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        })
    }

    #[tokio::test]
    async fn checks_github_release_and_reads_checksums() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        let asset_name = current_asset_name("0.4.0");
        let assets = [
            "macos-aarch64.zip",
            "macos-x86_64.zip",
            "windows-x86_64.zip",
            "linux-x86_64.tar.gz",
        ]
        .into_iter()
        .map(|suffix| asset_json(&base, &format!("RMCL-0.4.0-{suffix}")))
        .collect::<Vec<_>>();
        let mut assets = assets;
        assets.push(asset_json(&base, "SHA256SUMS"));
        let checksums = assets
            .iter()
            .filter(|asset| asset["name"].as_str() != Some("SHA256SUMS"))
            .map(|asset| format!("{}  {}", "a".repeat(64), asset["name"].as_str().unwrap()))
            .collect::<Vec<_>>()
            .join("\n");
        let release = json!({
            "tag_name": "v0.4.0",
            "name": "RMCL 0.4.0",
            "body": "Notes",
            "html_url": "https://github.com/driftcell/RMCL/releases/tag/v0.4.0",
            "draft": false,
            "prerelease": false,
            "assets": assets
        });
        let router = Router::new()
            .route(
                "/releases/latest",
                get({
                    let release = release.clone();
                    move || {
                        let release = release.clone();
                        async move { Json(release) }
                    }
                }),
            )
            .route(
                "/assets/SHA256SUMS",
                get({
                    let checksums = checksums.clone();
                    move || {
                        let checksums = checksums.clone();
                        async move { checksums }
                    }
                }),
            );
        let handle = serve(listener, router).await;
        let client = UpdateClient::new(base.join("releases/latest").unwrap()).unwrap();
        let release = client
            .check("0.3.0", &CancellationToken::new())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(release.version.to_string(), "0.4.0");
        assert_eq!(release.asset.name, asset_name);
        assert_eq!(release.sha256, "a".repeat(64));
        handle.abort();
    }

    #[tokio::test]
    async fn downloads_and_verifies_an_update_package() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        let bytes = b"update package".to_vec();
        let hash = format!("{:x}", Sha256::digest(&bytes));
        let router = Router::new().route(
            "/asset",
            get({
                let bytes = bytes.clone();
                move || {
                    let bytes = bytes.clone();
                    async move { bytes }
                }
            }),
        );
        let handle = serve(listener, router).await;
        let release = UpdateRelease {
            version: Version::parse("0.4.0").unwrap(),
            tag: "v0.4.0".to_owned(),
            title: "RMCL".to_owned(),
            notes: None,
            page_url: base.join("release").unwrap(),
            asset: UpdateAsset {
                name: "RMCL-0.4.0-linux-x86_64.tar.gz".to_owned(),
                url: base.join("asset").unwrap(),
                size: Some(bytes.len() as u64),
                package: UpdatePackage::LinuxTarGz,
            },
            sha256: hash,
        };
        let cache = tempfile::TempDir::new().unwrap();
        let client = UpdateClient::new(base.join("api").unwrap()).unwrap();
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let path = client
            .download(
                &release,
                cache.path(),
                Some(sender),
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        let progress = receiver.recv().await.unwrap();
        assert!(progress.total.is_some());
        handle.abort();
    }

    #[test]
    fn parses_checksums() {
        let checksums = "\
aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  RMCL-1.0.0-linux-x86_64.tar.gz
bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb *RMCL-1.0.0-windows-x86_64.zip
";
        assert_eq!(
            checksum_for(checksums, "RMCL-1.0.0-windows-x86_64.zip").as_deref(),
            Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
        );
        assert!(checksum_for(checksums, "missing").is_none());
    }

    #[test]
    fn compares_versions() {
        assert!(parse_version("v0.3.0").unwrap() < parse_version("0.4.0").unwrap());
        assert!(parse_version("0.4.0").is_ok());
        assert!(parse_version("not-a-version").is_err());
    }

    #[ignore = "queries the live GitHub Release API"]
    #[tokio::test]
    async fn checks_the_official_github_release() {
        let client = UpdateClient::github().unwrap();
        let release = client
            .check("0.2.0", &CancellationToken::new())
            .await
            .unwrap()
            .expect("the official repository has a release newer than 0.2.0");
        assert!(release.version > Version::parse("0.2.0").unwrap());
        assert!(!release.sha256.is_empty());
    }

    #[test]
    fn extracts_a_zip_update_package() {
        use std::io::Write as _;

        use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

        let temporary = tempfile::TempDir::new().unwrap();
        let archive = temporary.path().join("update.zip");
        let file = File::create(&archive).unwrap();
        let mut zip = ZipWriter::new(file);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        zip.start_file("RMCL-0.4.0/rmcl", options).unwrap();
        zip.write_all(b"binary").unwrap();
        zip.finish().unwrap();

        let destination = temporary.path().join("extracted");
        extract_package(&archive, UpdatePackage::WindowsZip, &destination).unwrap();
        assert_eq!(
            find_file(&destination, "rmcl").unwrap(),
            destination.join("extracted/RMCL-0.4.0/rmcl")
        );
    }

    #[test]
    fn replaces_a_file_and_cleans_the_backup() {
        let temporary = tempfile::TempDir::new().unwrap();
        let target = temporary.path().join("rmcl");
        let new = temporary.path().join("new-rmcl");
        fs::write(&target, b"old").unwrap();
        fs::write(&new, b"new").unwrap();
        replace_file(&new, &target).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new");
        assert!(!target.with_extension("old").exists());
    }
}
