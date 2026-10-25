use std::{
    io::Write,
    path::{Path, PathBuf},
};

use rmcl_install::resolve_version;
use rmcl_instance::{
    InstanceLoader, InstanceManifest, IsolationMode, LoaderKind, MinecraftLayout, VersionId,
};
use rmcl_java::JavaRequirement;
use rmcl_minecraft::Platform;
use tempfile::NamedTempFile;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::{
    AppError, ForgeInstallRequest, ForgeInstaller, InstallEvent, InstallOutcome, JavaService,
    LoaderCatalog, VanillaInstallRequest, VanillaInstaller, VersionSource,
};

/// Loader selected for a new instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoaderSelection {
    pub kind: LoaderKind,
    pub version: String,
    pub installer_version: String,
}

/// Request to create an isolated instance from a base Minecraft version.
#[derive(Debug, Clone)]
pub struct CreateInstanceRequest {
    pub minecraft_root: PathBuf,
    pub instance_id: String,
    pub name: String,
    pub base_version_id: String,
    pub platform: Platform,
    pub loader: Option<LoaderSelection>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstanceCreatePhase {
    Preparing,
    InstallingBase,
    FetchingLoader,
    WritingInstance,
    InstallingLoader,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstanceCreateEvent {
    Phase(InstanceCreatePhase),
    Install(InstallEvent),
    Processor {
        current: String,
        completed: usize,
        total: usize,
    },
}

pub type InstanceCreateEventSender = mpsc::Sender<InstanceCreateEvent>;

#[derive(Debug, Clone)]
pub struct InstanceCreateOutcome {
    pub install: InstallOutcome,
}

/// Creates an isolated instance and optionally installs a Fabric/Quilt loader
/// profile before planning the combined game files.
#[derive(Clone)]
pub struct InstanceCreator {
    installer: VanillaInstaller,
    loaders: LoaderCatalog,
    forge: Option<ForgeInstaller>,
    java: Option<JavaService>,
}

impl InstanceCreator {
    pub fn new(installer: VanillaInstaller, loaders: LoaderCatalog) -> Self {
        Self {
            installer,
            loaders,
            forge: None,
            java: None,
        }
    }

    pub fn with_forge(mut self, forge: ForgeInstaller, java: JavaService) -> Self {
        self.forge = Some(forge);
        self.java = Some(java);
        self
    }

    pub async fn create(
        &self,
        request: CreateInstanceRequest,
        cancellation: CancellationToken,
        events: Option<InstanceCreateEventSender>,
    ) -> Result<InstanceCreateOutcome, AppError> {
        check_cancelled(&cancellation)?;
        emit_phase(&events, InstanceCreatePhase::Preparing).await;
        let root = std::path::absolute(&request.minecraft_root).map_err(|source| AppError::Io {
            operation: "resolve Minecraft root",
            path: request.minecraft_root.clone(),
            source,
        })?;
        let layout = MinecraftLayout::new(root.clone());
        let base_id = VersionId::new(request.base_version_id.clone())?;
        let instance_id = VersionId::new(request.instance_id.clone())?;
        if request.loader.is_some() && instance_id == base_id {
            return Err(AppError::InvalidRequest(
                "a loader instance ID must differ from its base version ID".to_owned(),
            ));
        }
        if let Some(loader) = &request.loader
            && (loader.version.trim().is_empty() || loader.installer_version.trim().is_empty())
        {
            return Err(AppError::InvalidRequest(
                "loader version cannot be blank".to_owned(),
            ));
        }

        // The base version is the parent profile. When creating an instance
        // with the base ID itself, the normal Vanilla installer is the whole
        // operation and must not be overwritten with a synthetic child profile.
        if request.loader.is_none() && instance_id == base_id {
            emit_phase(&events, InstanceCreatePhase::InstallingBase).await;
            let outcome = self
                .install_with_events(
                    VanillaInstallRequest {
                        minecraft_root: root,
                        version_id: base_id.as_str().to_owned(),
                        platform: request.platform,
                        source: VersionSource::Manifest,
                    },
                    cancellation,
                    &events,
                )
                .await?;
            emit_phase(&events, InstanceCreatePhase::Completed).await;
            return Ok(InstanceCreateOutcome { install: outcome });
        }

        let base_json = layout.version_json_path(&base_id);
        let base_was_installed = !base_json.is_file();
        if base_was_installed {
            emit_phase(&events, InstanceCreatePhase::InstallingBase).await;
            self.install_with_events(
                VanillaInstallRequest {
                    minecraft_root: root.clone(),
                    version_id: base_id.as_str().to_owned(),
                    platform: request.platform.clone(),
                    source: VersionSource::Manifest,
                },
                cancellation.clone(),
                &events,
            )
            .await?;
        }
        let needs_forge = request
            .loader
            .as_ref()
            .is_some_and(|loader| matches!(loader.kind, LoaderKind::Forge | LoaderKind::NeoForge));
        if needs_forge && !base_was_installed {
            // Forge processors need the real client JAR before they run. Make
            // sure an existing base instance is complete instead of trusting
            // its JSON alone.
            emit_phase(&events, InstanceCreatePhase::InstallingBase).await;
            self.install_with_events(
                VanillaInstallRequest {
                    minecraft_root: root.clone(),
                    version_id: base_id.as_str().to_owned(),
                    platform: request.platform.clone(),
                    source: VersionSource::Existing,
                },
                cancellation.clone(),
                &events,
            )
            .await?;
        }

        let instance = layout.instance(instance_id.clone(), IsolationMode::Isolated);
        if instance.root().exists() {
            return Err(AppError::InvalidRequest(format!(
                "instance directory `{}` already exists",
                instance.root().display()
            )));
        }

        let profile = match &request.loader {
            Some(loader) if matches!(loader.kind, LoaderKind::Fabric | LoaderKind::Quilt) => {
                emit_phase(&events, InstanceCreatePhase::FetchingLoader).await;
                let profile = self
                    .loaders
                    .profile(
                        loader.kind,
                        base_id.as_str(),
                        &loader.version,
                        &cancellation,
                    )
                    .await?;
                rewrite_profile(&profile, instance_id.as_str(), base_id.as_str())?
            }
            Some(loader) => {
                emit_phase(&events, InstanceCreatePhase::FetchingLoader).await;
                let version_json = self
                    .forge_profile(
                        &layout,
                        &base_id,
                        loader,
                        &request.platform,
                        &cancellation,
                        &events,
                    )
                    .await?;
                rewrite_profile(&version_json, instance_id.as_str(), base_id.as_str())?
            }
            None => serde_json::json!({
                "id": instance_id.as_str(),
                "inheritsFrom": base_id.as_str(),
            })
            .to_string(),
        };

        check_cancelled(&cancellation)?;
        emit_phase(&events, InstanceCreatePhase::WritingInstance).await;
        let version_json_path = instance.version_json_path();
        write_atomic(&version_json_path, profile.as_bytes())?;
        let mut manifest = InstanceManifest::new(&instance_id, IsolationMode::Isolated);
        manifest.name = if request.name.trim().is_empty() {
            instance_id.as_str().to_owned()
        } else {
            request.name.trim().to_owned()
        };
        manifest.minecraft_version = base_id.as_str().to_owned();
        manifest.loader = request.loader.as_ref().map(|loader| InstanceLoader {
            kind: loader.kind,
            version: loader.version.clone(),
        });
        instance.ensure_content_dirs()?;
        if let Err(error) = instance.write_manifest(&manifest) {
            let _ = std::fs::remove_file(&version_json_path);
            return Err(error.into());
        }

        emit_phase(&events, InstanceCreatePhase::InstallingLoader).await;
        let outcome = self
            .install_with_events(
                VanillaInstallRequest {
                    minecraft_root: root,
                    version_id: instance_id.as_str().to_owned(),
                    platform: request.platform,
                    source: VersionSource::Existing,
                },
                cancellation,
                &events,
            )
            .await?;
        emit_phase(&events, InstanceCreatePhase::Completed).await;
        Ok(InstanceCreateOutcome { install: outcome })
    }

    async fn forge_profile(
        &self,
        layout: &MinecraftLayout,
        base_id: &VersionId,
        loader: &LoaderSelection,
        platform: &Platform,
        cancellation: &CancellationToken,
        events: &Option<InstanceCreateEventSender>,
    ) -> Result<String, AppError> {
        let forge = self
            .forge
            .as_ref()
            .ok_or(AppError::ForgeInstallNotConfigured)?;
        let java = self.java.as_ref().ok_or(AppError::JavaNotConfigured)?;
        let base_version = resolve_version(layout, base_id)?;
        let requirement = JavaRequirement::for_version(&base_version, platform);
        let selection = java
            .ensure(&requirement, cancellation.clone(), None)
            .await?;
        let installer_url =
            self.loaders
                .installer_url(loader.kind, base_id.as_str(), &loader.installer_version)?;
        let outcome = forge
            .install(
                ForgeInstallRequest {
                    minecraft_root: layout.root().to_path_buf(),
                    base_version_id: base_id.clone(),
                    base_version,
                    platform: platform.clone(),
                    java: selection.runtime,
                    installer_url,
                },
                cancellation.clone(),
                events,
            )
            .await?;
        Ok(outcome.version_json)
    }

    async fn install_with_events(
        &self,
        request: VanillaInstallRequest,
        cancellation: CancellationToken,
        events: &Option<InstanceCreateEventSender>,
    ) -> Result<InstallOutcome, AppError> {
        let (sender, mut receiver) = mpsc::channel(128);
        let install = Box::pin(self.installer.install(request, cancellation, Some(sender)));
        let forward = async {
            while let Some(event) = receiver.recv().await {
                emit(events, InstanceCreateEvent::Install(event)).await;
            }
        };
        let (result, ()) = tokio::join!(install, forward);
        result
    }
}

fn rewrite_profile(
    raw: &str,
    instance_id: &str,
    base_version_id: &str,
) -> Result<String, AppError> {
    let mut value: serde_json::Value =
        serde_json::from_str(raw).map_err(|error| AppError::LoaderMetadataInvalid {
            message: error.to_string(),
        })?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| AppError::LoaderMetadataInvalid {
            message: "loader profile is not a JSON object".to_owned(),
        })?;
    object.insert(
        "id".to_owned(),
        serde_json::Value::String(instance_id.to_owned()),
    );
    object.insert(
        "inheritsFrom".to_owned(),
        serde_json::Value::String(base_version_id.to_owned()),
    );
    serde_json::to_string_pretty(&value).map_err(|error| AppError::LoaderMetadataInvalid {
        message: error.to_string(),
    })
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let parent = path.parent().ok_or_else(|| AppError::Io {
        operation: "find instance version JSON directory",
        path: path.to_path_buf(),
        source: std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "instance version JSON path has no parent",
        ),
    })?;
    std::fs::create_dir_all(parent).map_err(|source| AppError::Io {
        operation: "create instance version JSON directory",
        path: parent.to_path_buf(),
        source,
    })?;
    let mut temporary = NamedTempFile::new_in(parent).map_err(|source| AppError::Io {
        operation: "create instance version JSON temporary file",
        path: parent.to_path_buf(),
        source,
    })?;
    temporary.write_all(bytes).map_err(|source| AppError::Io {
        operation: "write instance version JSON temporary file",
        path: temporary.path().to_path_buf(),
        source,
    })?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|source| AppError::Io {
            operation: "sync instance version JSON temporary file",
            path: temporary.path().to_path_buf(),
            source,
        })?;
    temporary.persist(path).map_err(|error| AppError::Io {
        operation: "publish instance version JSON",
        path: path.to_path_buf(),
        source: error.error,
    })?;
    Ok(())
}

fn check_cancelled(cancellation: &CancellationToken) -> Result<(), AppError> {
    if cancellation.is_cancelled() {
        Err(AppError::Cancelled)
    } else {
        Ok(())
    }
}

async fn emit_phase(events: &Option<InstanceCreateEventSender>, phase: InstanceCreatePhase) {
    emit(events, InstanceCreateEvent::Phase(phase)).await;
}

async fn emit(events: &Option<InstanceCreateEventSender>, event: InstanceCreateEvent) {
    if let Some(events) = events {
        let _ = events.send(event).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_profile_identity_without_dropping_fields() {
        let raw = r#"{
            "id": "fabric-loader-0.16.10-1.21.1",
            "inheritsFrom": "1.21.1",
            "mainClass": "example.Main",
            "libraries": [],
            "unknown": {"kept": true}
        }"#;

        let rewritten = rewrite_profile(raw, "my-instance", "1.21.1").unwrap();
        let value: serde_json::Value = serde_json::from_str(&rewritten).unwrap();

        assert_eq!(value["id"], "my-instance");
        assert_eq!(value["inheritsFrom"], "1.21.1");
        assert_eq!(value["mainClass"], "example.Main");
        assert_eq!(value["unknown"]["kept"], true);
    }
}
