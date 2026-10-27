use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use rmcl_archive::ArchiveExtractor;
use rmcl_domain::{DownloadRequest, ExtractionPlan, Sha1Hash};
use rmcl_install::{MavenCoordinate, plan_libraries};
use rmcl_instance::{MinecraftLayout, VersionId};
use rmcl_java::JavaRuntime;
use rmcl_minecraft::{DownloadInfo, GameVersion, Library, Platform};
use rmcl_net::{
    DownloadConfig, DownloadError, DownloadExecutor, DownloadReport, DownloadStatus, ProgressStage,
    progress_channel,
};
use serde::Deserialize;
use sha1::{Digest, Sha1};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;
use url::Url;
use walkdir::WalkDir;
use zip::ZipArchive;

use crate::{
    AppError, InstallDownloadFailure, InstallDownloadProgress, InstallEvent, InstallPhase,
    InstanceCreateEvent, InstanceCreateEventSender,
};

/// Request to run one Forge/NeoForge installer JAR.
#[derive(Debug, Clone)]
pub struct ForgeInstallRequest {
    pub minecraft_root: PathBuf,
    pub base_version_id: VersionId,
    pub base_version: GameVersion,
    pub platform: Platform,
    pub java: JavaRuntime,
    pub installer_url: Url,
}

#[derive(Debug, Clone)]
pub struct ForgeInstallOutcome {
    pub version_json: String,
    pub processor_count: usize,
}

/// Downloads and runs a Forge/NeoForge installer's processor-based client setup.
#[derive(Clone)]
pub struct ForgeInstaller {
    downloader: DownloadExecutor,
    extractor: ArchiveExtractor,
    metadata: reqwest::Client,
}

impl ForgeInstaller {
    pub fn new(config: DownloadConfig) -> Result<Self, AppError> {
        let metadata = reqwest::Client::builder()
            .user_agent(concat!(
                "RMCL/",
                env!("CARGO_PKG_VERSION"),
                " (https://github.com/driftcell/RMCL)"
            ))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|source| AppError::LoaderInstaller {
                message: format!("failed to build installer metadata client: {source}"),
            })?;
        Ok(Self {
            downloader: DownloadExecutor::new(config)?,
            extractor: ArchiveExtractor::default(),
            metadata,
        })
    }

    pub async fn install(
        &self,
        request: ForgeInstallRequest,
        cancellation: CancellationToken,
        events: &Option<InstanceCreateEventSender>,
    ) -> Result<ForgeInstallOutcome, AppError> {
        let root = std::path::absolute(&request.minecraft_root).map_err(|source| AppError::Io {
            operation: "resolve Minecraft root",
            path: request.minecraft_root.clone(),
            source,
        })?;
        let layout = MinecraftLayout::new(root.clone());
        let client_jar = layout.client_jar_path(&request.base_version_id);
        if !client_jar.is_file() {
            return Err(AppError::InvalidRequest(format!(
                "base client JAR `{}` is missing before Forge processors",
                client_jar.display()
            )));
        }
        fs::create_dir_all(layout.libraries_dir()).map_err(|source| AppError::Io {
            operation: "create libraries directory",
            path: layout.libraries_dir(),
            source,
        })?;

        emit_phase(events, crate::InstanceCreatePhase::InstallingLoader).await;
        let temporary = tempfile::tempdir().map_err(|source| AppError::Io {
            operation: "create Forge installer temporary directory",
            path: std::env::temp_dir(),
            source,
        })?;
        let installer = temporary.path().join("installer.jar");
        let installer_sha1 = self
            .installer_sha1(&request.installer_url, &cancellation)
            .await?;
        self.download_single(
            DownloadRequest {
                sources: vec![request.installer_url.clone()],
                destination: installer.clone(),
                sha1: installer_sha1,
                size: None,
            },
            InstallPhase::DownloadingLoader,
            &cancellation,
            events,
        )
        .await?;

        let extracted = temporary.path().join("installer");
        self.extractor
            .extract_zip(
                &ExtractionPlan {
                    source: installer.clone(),
                    destination: extracted.clone(),
                    exclude_prefixes: Vec::new(),
                },
                cancellation.clone(),
                None,
            )
            .map_err(|source| match source {
                rmcl_archive::ExtractionError::Cancelled => AppError::Cancelled,
                source => AppError::LoaderInstaller {
                    message: source.to_string(),
                },
            })?;

        let install_profile_path = extracted.join("install_profile.json");
        let profile_text = fs::read_to_string(&install_profile_path).map_err(|source| {
            AppError::LoaderInstaller {
                message: format!(
                    "failed to read `{}`: {source}",
                    install_profile_path.display()
                ),
            }
        })?;
        let raw: serde_json::Value =
            serde_json::from_str(&profile_text).map_err(|source| AppError::LoaderInstaller {
                message: format!("invalid install_profile.json: {source}"),
            })?;
        if raw.get("spec").is_none() {
            return Err(AppError::LoaderInstaller {
                message: "legacy Forge/NeoForge installer format is not supported yet".to_owned(),
            });
        }
        let profile: InstallProfile =
            serde_json::from_value(raw).map_err(|source| AppError::LoaderInstaller {
                message: format!("invalid install_profile.json: {source}"),
            })?;
        if profile.minecraft != request.base_version_id.as_str() {
            return Err(AppError::LoaderInstaller {
                message: format!(
                    "installer targets Minecraft `{}`, expected `{}`",
                    profile.minecraft, request.base_version_id
                ),
            });
        }

        let version_relative =
            safe_relative(&profile.json).ok_or_else(|| AppError::LoaderInstaller {
                message: format!("unsafe version JSON path `{}`", profile.json),
            })?;
        let version_path = extracted.join(version_relative);
        let version_json =
            fs::read_to_string(&version_path).map_err(|source| AppError::LoaderInstaller {
                message: format!("failed to read `{}`: {source}", version_path.display()),
            })?;
        GameVersion::from_json(&version_json).map_err(|source| AppError::LoaderInstaller {
            message: format!("invalid embedded version JSON: {source}"),
        })?;

        let copied = copy_embedded_maven(&extracted.join("maven"), &layout.libraries_dir())?;
        let library_plan = plan_libraries(&layout, &profile.libraries, &request.platform)?;
        let library_plan = rmcl_install::InstallPlan {
            files: library_plan
                .files
                .into_iter()
                .filter(|file| !copied.contains(&file.destination))
                .collect(),
            extractions: Vec::new(),
        };
        self.download_plan(
            library_plan,
            InstallPhase::DownloadingLoader,
            &cancellation,
            events,
        )
        .await?;

        let data_directory = temporary.path().join("data");
        fs::create_dir_all(&data_directory).map_err(|source| AppError::Io {
            operation: "create Forge data directory",
            path: data_directory.clone(),
            source,
        })?;
        let mut variables = BTreeMap::new();
        for (key, value) in &profile.data {
            let resolved = resolve_data_value(
                value.client(),
                &variables,
                &extracted,
                &layout,
                &data_directory,
            )?;
            variables.insert(key.clone(), resolved);
        }
        variables.insert("SIDE".to_owned(), "client".to_owned());
        variables.insert("MINECRAFT_JAR".to_owned(), absolute_string(&client_jar)?);
        variables.insert(
            "MINECRAFT_VERSION".to_owned(),
            request.base_version_id.as_str().to_owned(),
        );
        variables.insert("ROOT".to_owned(), absolute_string(&root)?);
        variables.insert("INSTALLER".to_owned(), absolute_string(&installer)?);
        variables.insert(
            "LIBRARY_DIR".to_owned(),
            absolute_string(&layout.libraries_dir())?,
        );

        let processors = profile
            .processors
            .iter()
            .filter(|processor| {
                processor.sides.is_empty() || processor.sides.iter().any(|side| side == "client")
            })
            .collect::<Vec<_>>();
        let total = processors.len();
        emit_processor(events, "准备处理器".to_owned(), 0, total).await;
        for (index, processor) in processors.iter().enumerate() {
            if cancellation.is_cancelled() {
                return Err(AppError::Cancelled);
            }
            let args = processor
                .args
                .iter()
                .map(|argument| parse_literal(argument, &variables, &layout))
                .collect::<Result<Vec<_>, _>>()?;
            if self
                .handle_mojmaps(&args, &request, &layout, &cancellation, events)
                .await?
            {
                emit_processor(events, processor.jar.clone(), index + 1, total).await;
                continue;
            }

            let outputs = processor_outputs(processor, &variables, &layout)?;
            if !outputs.is_empty() && outputs.iter().all(|(path, hash)| file_matches(path, hash)) {
                emit_processor(events, processor.jar.clone(), index + 1, total).await;
                continue;
            }
            for path in outputs.keys() {
                if path.exists() {
                    fs::remove_file(path).map_err(|source| AppError::Io {
                        operation: "remove stale processor output",
                        path: path.clone(),
                        source,
                    })?;
                }
            }

            let jar = library_path(&layout, &processor.jar)?;
            let main_class = main_class_from_jar(&jar)?;
            let mut classpath = processor
                .classpath
                .iter()
                .map(|entry| library_path(&layout, entry))
                .collect::<Result<Vec<_>, _>>()?;
            classpath.push(jar);
            run_processor(
                &request.java.executable,
                &classpath,
                &main_class,
                &args,
                &data_directory,
                &cancellation,
            )
            .await?;
            verify_outputs(&outputs)?;
            emit_processor(events, processor.jar.clone(), index + 1, total).await;
        }

        Ok(ForgeInstallOutcome {
            version_json,
            processor_count: total,
        })
    }

    async fn handle_mojmaps(
        &self,
        args: &[String],
        request: &ForgeInstallRequest,
        layout: &MinecraftLayout,
        cancellation: &CancellationToken,
        events: &Option<InstanceCreateEventSender>,
    ) -> Result<bool, AppError> {
        if option_value(args, "--task") != Some("DOWNLOAD_MOJMAPS") {
            return Ok(false);
        }
        let output = option_value(args, "--output").ok_or_else(|| AppError::LoaderInstaller {
            message: "DOWNLOAD_MOJMAPS processor has no output".to_owned(),
        })?;
        let output = PathBuf::from(output);
        ensure_under_libraries(&output, &layout.libraries_dir())?;
        let mappings = request
            .base_version
            .downloads
            .get("client_mappings")
            .ok_or_else(|| AppError::LoaderInstaller {
                message: "base version has no client_mappings download".to_owned(),
            })?;
        let mappings = mappings_from_info(mappings, output)?;
        self.download_single(
            mappings,
            InstallPhase::DownloadingLoader,
            cancellation,
            events,
        )
        .await?;
        Ok(true)
    }

    /// Maven publishes a `.sha1` sidecar next to Forge/NeoForge installers.
    /// A missing sidecar is tolerated for mirrors and test fixtures; a present
    /// but malformed or failed sidecar aborts the install rather than silently
    /// skipping integrity verification.
    async fn installer_sha1(
        &self,
        url: &Url,
        cancellation: &CancellationToken,
    ) -> Result<Option<Sha1Hash>, AppError> {
        let sha_url =
            Url::parse(&format!("{url}.sha1")).map_err(|source| AppError::LoaderInstaller {
                message: format!("invalid installer SHA-1 URL: {source}"),
            })?;
        let response = tokio::select! {
            _ = cancellation.cancelled() => return Err(AppError::Cancelled),
            response = self.metadata.get(sha_url).send() => response.map_err(|source| {
                AppError::LoaderInstaller {
                    message: format!("failed to fetch installer SHA-1: {source}"),
                }
            })?,
        };
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status().is_success() {
            return Err(AppError::LoaderInstaller {
                message: format!(
                    "installer SHA-1 request failed with HTTP {}",
                    response.status().as_u16()
                ),
            });
        }
        let text = tokio::select! {
            _ = cancellation.cancelled() => return Err(AppError::Cancelled),
            text = response.text() => text.map_err(|source| AppError::LoaderInstaller {
                message: format!("failed to read installer SHA-1: {source}"),
            })?,
        };
        let value = text
            .split_whitespace()
            .next()
            .ok_or_else(|| AppError::LoaderInstaller {
                message: "installer SHA-1 response is empty".to_owned(),
            })?;
        value
            .parse()
            .map(Some)
            .map_err(|source| AppError::LoaderInstaller {
                message: format!("invalid installer SHA-1 `{value}`: {source}"),
            })
    }

    async fn download_single(
        &self,
        request: DownloadRequest,
        phase: InstallPhase,
        cancellation: &CancellationToken,
        events: &Option<InstanceCreateEventSender>,
    ) -> Result<DownloadStatus, AppError> {
        let (progress, mut receiver) = progress_channel(128);
        let download =
            self.downloader
                .download_file(&request, cancellation.clone(), Some(progress));
        let forward = async {
            while let Some(event) = receiver.recv().await {
                let completed = matches!(
                    event.stage,
                    ProgressStage::Completed
                        | ProgressStage::Skipped
                        | ProgressStage::Failed
                        | ProgressStage::Cancelled
                );
                emit(
                    events,
                    InstanceCreateEvent::Install(InstallEvent::Download(InstallDownloadProgress {
                        phase,
                        stage: event.stage,
                        current: event.destination,
                        completed_files: usize::from(completed),
                        total_files: 1,
                        downloaded_bytes: event.downloaded_bytes,
                        total_bytes: event.total_bytes,
                        detail: event.detail,
                    })),
                )
                .await;
            }
        };
        let (result, ()) = tokio::join!(download, forward);
        result.map_err(|source| loader_download_error(source, cancellation))
    }

    async fn download_plan(
        &self,
        plan: rmcl_install::InstallPlan,
        phase: InstallPhase,
        cancellation: &CancellationToken,
        events: &Option<InstanceCreateEventSender>,
    ) -> Result<(), AppError> {
        let requests = plan
            .files
            .iter()
            .filter(|file| !file.sources.is_empty())
            .map(DownloadRequest::from)
            .collect::<Vec<_>>();
        if requests.is_empty() {
            return Ok(());
        }
        let total_files = requests.len();
        let (progress, mut receiver) = progress_channel(128);
        let download = self
            .downloader
            .download_all(requests, cancellation.clone(), Some(progress));
        let forward = async {
            let mut completed = HashSet::new();
            while let Some(event) = receiver.recv().await {
                if matches!(
                    event.stage,
                    ProgressStage::Completed
                        | ProgressStage::Skipped
                        | ProgressStage::Failed
                        | ProgressStage::Cancelled
                ) {
                    completed.insert(event.destination.clone());
                }
                emit(
                    events,
                    InstanceCreateEvent::Install(InstallEvent::Download(InstallDownloadProgress {
                        phase,
                        stage: event.stage,
                        current: event.destination,
                        completed_files: completed.len(),
                        total_files,
                        downloaded_bytes: event.downloaded_bytes,
                        total_bytes: event.total_bytes,
                        detail: event.detail,
                    })),
                )
                .await;
            }
        };
        let (report, ()) = tokio::join!(download, forward);
        validate_report(report, phase, cancellation)
    }
}

fn loader_download_error(source: DownloadError, cancellation: &CancellationToken) -> AppError {
    if cancellation.is_cancelled() || matches!(source, DownloadError::Cancelled) {
        AppError::Cancelled
    } else {
        AppError::LoaderDownload { source }
    }
}

fn validate_report(
    report: DownloadReport,
    phase: InstallPhase,
    cancellation: &CancellationToken,
) -> Result<(), AppError> {
    if report.cancelled || cancellation.is_cancelled() {
        return Err(AppError::Cancelled);
    }
    if !report.failures.is_empty() {
        return Err(AppError::DownloadBatch {
            phase,
            failures: report
                .failures
                .into_iter()
                .map(|failure| InstallDownloadFailure {
                    destination: failure.request.destination,
                    message: failure.error.to_string(),
                })
                .collect(),
        });
    }
    Ok(())
}

fn mappings_from_info(
    info: &DownloadInfo,
    destination: PathBuf,
) -> Result<DownloadRequest, AppError> {
    let url = info
        .url
        .as_deref()
        .filter(|url| !url.trim().is_empty())
        .ok_or_else(|| AppError::LoaderInstaller {
            message: "client_mappings has no URL".to_owned(),
        })?;
    let url = Url::parse(url).map_err(|source| AppError::LoaderInstaller {
        message: format!("invalid client_mappings URL: {source}"),
    })?;
    Ok(DownloadRequest {
        sources: vec![url],
        destination,
        sha1: info
            .sha1
            .as_deref()
            .map(str::parse)
            .transpose()
            .map_err(|source| AppError::LoaderInstaller {
                message: format!("invalid client_mappings SHA-1: {source}"),
            })?,
        size: info.size,
    })
}

fn resolve_data_value(
    value: &str,
    variables: &BTreeMap<String, String>,
    installer_root: &Path,
    layout: &MinecraftLayout,
    data_directory: &Path,
) -> Result<String, AppError> {
    let trimmed = value.trim();
    if let Some(key) = surrounded(trimmed, '{', '}') {
        return variables
            .get(key)
            .cloned()
            .ok_or_else(|| AppError::LoaderInstaller {
                message: format!("loader data references unknown variable `{key}`"),
            });
    }
    if let Some(literal) = surrounded(trimmed, '\'', '\'') {
        return Ok(literal.to_owned());
    }
    if let Some(coordinate) = surrounded(trimmed, '[', ']') {
        return Ok(library_path(layout, coordinate)?.display().to_string());
    }

    let relative = safe_relative(trimmed).ok_or_else(|| AppError::LoaderInstaller {
        message: format!("unsafe installer data path `{value}`"),
    })?;
    let source = installer_root.join(&relative);
    if !source.is_file() {
        return Err(AppError::LoaderInstaller {
            message: format!("installer data file `{}` is missing", source.display()),
        });
    }
    let destination = data_directory.join(relative);
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|source| AppError::Io {
            operation: "create installer data directory",
            path: parent.to_path_buf(),
            source,
        })?;
    }
    fs::copy(&source, &destination).map_err(|source| AppError::Io {
        operation: "copy installer data file",
        path: destination.clone(),
        source,
    })?;
    absolute_string(&destination)
}

fn processor_outputs(
    processor: &Processor,
    variables: &BTreeMap<String, String>,
    layout: &MinecraftLayout,
) -> Result<BTreeMap<PathBuf, rmcl_domain::Sha1Hash>, AppError> {
    let mut outputs = BTreeMap::new();
    for (path, hash) in &processor.outputs {
        let path = PathBuf::from(parse_literal(path, variables, layout)?);
        ensure_under_libraries(&path, &layout.libraries_dir())?;
        let hash = parse_literal(hash, variables, layout)?
            .parse()
            .map_err(|source| AppError::LoaderInstaller {
                message: format!("invalid processor output SHA-1: {source}"),
            })?;
        outputs.insert(path, hash);
    }
    Ok(outputs)
}

fn parse_literal(
    value: &str,
    variables: &BTreeMap<String, String>,
    layout: &MinecraftLayout,
) -> Result<String, AppError> {
    let trimmed = value.trim();
    if let Some(key) = surrounded(trimmed, '{', '}') {
        return variables
            .get(key)
            .cloned()
            .ok_or_else(|| AppError::LoaderInstaller {
                message: format!("loader processor references unknown variable `{key}`"),
            });
    }
    if let Some(literal) = surrounded(trimmed, '\'', '\'') {
        return Ok(literal.to_owned());
    }
    if let Some(coordinate) = surrounded(trimmed, '[', ']') {
        return Ok(library_path(layout, coordinate)?.display().to_string());
    }
    replace_tokens(trimmed, variables)
}

fn replace_tokens(value: &str, variables: &BTreeMap<String, String>) -> Result<String, AppError> {
    let characters = value.chars().collect::<Vec<_>>();
    let mut output = String::with_capacity(value.len());
    let mut index = 0;
    while index < characters.len() {
        match characters[index] {
            '\\' => {
                index += 1;
                let character = characters
                    .get(index)
                    .ok_or_else(|| AppError::LoaderInstaller {
                        message: format!("bad escape in loader literal `{value}`"),
                    })?;
                output.push(*character);
                index += 1;
            }
            '{' => {
                index += 1;
                let start = index;
                while index < characters.len() && characters[index] != '}' {
                    index += 1;
                }
                if index == characters.len() {
                    return Err(AppError::LoaderInstaller {
                        message: format!("unclosed variable in loader literal `{value}`"),
                    });
                }
                let key = characters[start..index].iter().collect::<String>();
                let replacement = variables
                    .get(&key)
                    .ok_or_else(|| AppError::LoaderInstaller {
                        message: format!("loader processor references unknown variable `{key}`"),
                    })?;
                output.push_str(replacement);
                index += 1;
            }
            '\'' => {
                index += 1;
                while index < characters.len() && characters[index] != '\'' {
                    if characters[index] == '\\' {
                        index += 1;
                        let character =
                            characters
                                .get(index)
                                .ok_or_else(|| AppError::LoaderInstaller {
                                    message: format!("bad escape in loader literal `{value}`"),
                                })?;
                        output.push(*character);
                    } else {
                        output.push(characters[index]);
                    }
                    index += 1;
                }
                if index == characters.len() {
                    return Err(AppError::LoaderInstaller {
                        message: format!("unclosed quote in loader literal `{value}`"),
                    });
                }
                index += 1;
            }
            character => {
                output.push(character);
                index += 1;
            }
        }
    }
    Ok(output)
}

fn surrounded(value: &str, start: char, end: char) -> Option<&str> {
    value
        .strip_prefix(start)
        .and_then(|value| value.strip_suffix(end))
}

fn option_value<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter()
        .position(|argument| argument == name)
        .and_then(|index| args.get(index + 1))
        .map(String::as_str)
}

fn library_path(layout: &MinecraftLayout, coordinate: &str) -> Result<PathBuf, AppError> {
    let parsed =
        MavenCoordinate::parse(coordinate).map_err(|source| AppError::LoaderInstaller {
            message: format!("invalid Maven coordinate `{coordinate}`: {source}"),
        })?;
    let path = parsed
        .repository_path()
        .map_err(|source| AppError::LoaderInstaller {
            message: format!("invalid Maven coordinate `{coordinate}`: {source}"),
        })?;
    Ok(layout.library_path(&path))
}

fn safe_relative(value: &str) -> Option<PathBuf> {
    let value = value.strip_prefix('/').unwrap_or(value);
    let path = Path::new(value);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
    {
        return None;
    }
    Some(path.to_path_buf())
}

fn ensure_under_libraries(path: &Path, libraries: &Path) -> Result<(), AppError> {
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(AppError::LoaderInstaller {
            message: format!("unsafe processor output path `{}`", path.display()),
        });
    }
    let absolute = std::path::absolute(path).map_err(|source| AppError::Io {
        operation: "resolve processor output",
        path: path.to_path_buf(),
        source,
    })?;
    let libraries = std::path::absolute(libraries).map_err(|source| AppError::Io {
        operation: "resolve libraries directory",
        path: libraries.to_path_buf(),
        source,
    })?;
    if !absolute.starts_with(&libraries) {
        return Err(AppError::LoaderInstaller {
            message: format!(
                "processor output `{}` is outside the libraries directory",
                path.display()
            ),
        });
    }
    Ok(())
}

fn copy_embedded_maven(source: &Path, libraries: &Path) -> Result<HashSet<PathBuf>, AppError> {
    let mut copied = HashSet::new();
    if !source.is_dir() {
        return Ok(copied);
    }
    for entry in WalkDir::new(source).follow_links(false) {
        let entry = entry.map_err(|source| AppError::LoaderInstaller {
            message: format!("failed to inspect embedded maven files: {source}"),
        })?;
        if entry.file_type().is_dir() {
            continue;
        }
        if entry.file_type().is_symlink() || !entry.file_type().is_file() {
            continue;
        }
        let relative =
            entry
                .path()
                .strip_prefix(source)
                .map_err(|_| AppError::LoaderInstaller {
                    message: "embedded maven path escaped the installer".to_owned(),
                })?;
        let destination = libraries.join(relative);
        if destination.exists() {
            continue;
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|source| AppError::Io {
                operation: "create embedded library directory",
                path: parent.to_path_buf(),
                source,
            })?;
        }
        fs::copy(entry.path(), &destination).map_err(|source| AppError::Io {
            operation: "copy embedded library",
            path: destination.clone(),
            source,
        })?;
        copied.insert(destination);
    }
    Ok(copied)
}

fn main_class_from_jar(path: &Path) -> Result<String, AppError> {
    let file = fs::File::open(path).map_err(|source| AppError::Io {
        operation: "open processor JAR",
        path: path.to_path_buf(),
        source,
    })?;
    let mut archive = ZipArchive::new(file).map_err(|source| AppError::LoaderProcessor {
        message: format!(
            "processor `{}` is not a valid JAR: {source}",
            path.display()
        ),
    })?;
    let mut manifest =
        archive
            .by_name("META-INF/MANIFEST.MF")
            .map_err(|source| AppError::LoaderProcessor {
                message: format!(
                    "processor `{}` has no META-INF/MANIFEST.MF: {source}",
                    path.display()
                ),
            })?;
    let mut text = String::new();
    manifest
        .read_to_string(&mut text)
        .map_err(|source| AppError::LoaderProcessor {
            message: format!(
                "failed to read processor manifest `{}`: {source}",
                path.display()
            ),
        })?;
    text.lines()
        .find_map(|line| line.strip_prefix("Main-Class:"))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| AppError::LoaderProcessor {
            message: format!("processor `{}` has no Main-Class", path.display()),
        })
}

async fn run_processor(
    java: &Path,
    classpath: &[PathBuf],
    main_class: &str,
    args: &[String],
    working_directory: &Path,
    cancellation: &CancellationToken,
) -> Result<(), AppError> {
    let classpath =
        std::env::join_paths(classpath).map_err(|source| AppError::LoaderProcessor {
            message: format!("invalid processor classpath: {source}"),
        })?;
    let mut command = Command::new(java);
    command
        .current_dir(working_directory)
        .arg("-cp")
        .arg(classpath)
        .arg(main_class)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let child = command.spawn().map_err(|source| AppError::Io {
        operation: "start loader processor",
        path: java.to_path_buf(),
        source,
    })?;
    let output = tokio::select! {
        _ = cancellation.cancelled() => return Err(AppError::Cancelled),
        output = child.wait_with_output() => output.map_err(|source| AppError::LoaderProcessor {
            message: format!("failed to wait for loader processor: {source}"),
        })?,
    };
    if output.status.success() {
        return Ok(());
    }
    let mut message = String::new();
    message.push_str(&String::from_utf8_lossy(&output.stdout));
    message.push_str(&String::from_utf8_lossy(&output.stderr));
    let tail = message
        .chars()
        .rev()
        .take(4000)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    Err(AppError::LoaderProcessor {
        message: format!(
            "`{main_class}` exited with {:?}: {tail}",
            output.status.code()
        ),
    })
}

fn verify_outputs(outputs: &BTreeMap<PathBuf, rmcl_domain::Sha1Hash>) -> Result<(), AppError> {
    for (path, expected) in outputs {
        if !path.is_file() {
            return Err(AppError::LoaderProcessor {
                message: format!("processor did not create `{}`", path.display()),
            });
        }
        if file_matches(path, expected) {
            continue;
        }
        if path.extension().is_some_and(|extension| extension == "jar") && is_valid_zip(path) {
            // Forge/NeoForge may generate JARs with zlib-compatible but
            // byte-different compression. Structural validation is the
            // fallback used by other launchers for these generated artifacts.
            continue;
        }
        let actual = sha1_file(path)?;
        let _ = fs::remove_file(path);
        return Err(AppError::LoaderProcessor {
            message: format!(
                "processor output `{}` has SHA-1 {actual}, expected {expected}",
                path.display()
            ),
        });
    }
    Ok(())
}

fn file_matches(path: &Path, expected: &rmcl_domain::Sha1Hash) -> bool {
    path.is_file() && sha1_file(path).is_ok_and(|actual| actual == *expected)
}

fn sha1_file(path: &Path) -> Result<rmcl_domain::Sha1Hash, AppError> {
    let mut file = fs::File::open(path).map_err(|source| AppError::Io {
        operation: "open processor output",
        path: path.to_path_buf(),
        source,
    })?;
    let mut hasher = Sha1::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|source| AppError::Io {
            operation: "hash processor output",
            path: path.to_path_buf(),
            source,
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let digest = hex::encode(hasher.finalize());
    digest.parse().map_err(|source| AppError::LoaderInstaller {
        message: format!("invalid computed SHA-1: {source}"),
    })
}

fn is_valid_zip(path: &Path) -> bool {
    fs::File::open(path)
        .ok()
        .and_then(|file| ZipArchive::new(file).ok())
        .is_some()
}

fn absolute_string(path: &Path) -> Result<String, AppError> {
    std::path::absolute(path)
        .map(|path| path.display().to_string())
        .map_err(|source| AppError::Io {
            operation: "resolve installer path",
            path: path.to_path_buf(),
            source,
        })
}

async fn emit_phase(events: &Option<InstanceCreateEventSender>, phase: crate::InstanceCreatePhase) {
    emit(events, InstanceCreateEvent::Phase(phase)).await;
}

async fn emit_processor(
    events: &Option<InstanceCreateEventSender>,
    current: String,
    completed: usize,
    total: usize,
) {
    emit(
        events,
        InstanceCreateEvent::Processor {
            current,
            completed,
            total,
        },
    )
    .await;
}

async fn emit(events: &Option<InstanceCreateEventSender>, event: InstanceCreateEvent) {
    if let Some(events) = events {
        let _ = events.send(event).await;
    }
}

#[derive(Debug, Deserialize)]
struct InstallProfile {
    #[serde(default)]
    minecraft: String,
    #[serde(default)]
    json: String,
    #[serde(default)]
    libraries: Vec<Library>,
    #[serde(default)]
    processors: Vec<Processor>,
    #[serde(default)]
    data: BTreeMap<String, DataValue>,
}

#[derive(Debug, Deserialize)]
struct Processor {
    #[serde(default)]
    sides: Vec<String>,
    jar: String,
    #[serde(default)]
    classpath: Vec<String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    outputs: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum DataValue {
    Structured { client: String },
    Plain(String),
}

impl DataValue {
    fn client(&self) -> &str {
        match self {
            Self::Structured { client } => client,
            Self::Plain(value) => value,
        }
    }
}
