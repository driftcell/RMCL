use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use rmcl_domain::Platform;
use rmcl_install::{InstallPlan, PlannedFileKind};
use rmcl_instance::{MinecraftLayout, VersionId};
use rmcl_java::JavaRuntime;
use rmcl_minecraft::{FeatureMap, GameVersion};

use crate::{LaunchAccount, LaunchError};

#[derive(Debug, Clone)]
pub struct LaunchOptions {
    pub java: JavaRuntime,
    pub account: LaunchAccount,
    pub platform: Platform,
    pub game_directory: PathBuf,
    pub assets_root: PathBuf,
    pub natives_directory: PathBuf,
    pub client_jar: PathBuf,
    pub libraries: Vec<PathBuf>,
    pub additional_jvm_arguments: Vec<String>,
    pub additional_game_arguments: Vec<String>,
    pub features: FeatureMap,
    pub resolution: Option<(u32, u32)>,
    pub demo: bool,
    pub environment: BTreeMap<String, String>,
    pub launcher_brand: String,
    pub launcher_version: String,
}

impl LaunchOptions {
    pub fn from_layout(
        layout: &MinecraftLayout,
        java: JavaRuntime,
        version: &GameVersion,
        account: LaunchAccount,
        platform: &Platform,
    ) -> Result<Self, LaunchError> {
        let layout = MinecraftLayout::new(absolute_path(layout.root())?);
        let id = VersionId::new(version.id.clone())?;
        let jar_id = VersionId::new(version.jar.as_deref().unwrap_or(&version.id))?;
        let mut features = FeatureMap::new();
        features.insert("has_custom_resolution".to_owned(), false);
        features.insert("is_demo_user".to_owned(), false);
        Ok(Self {
            java,
            account,
            platform: platform.clone(),
            game_directory: layout.root().to_path_buf(),
            assets_root: layout.assets_dir(),
            natives_directory: layout.natives_dir(&id, platform),
            client_jar: layout.client_jar_path(&jar_id),
            libraries: Vec::new(),
            additional_jvm_arguments: Vec::new(),
            additional_game_arguments: Vec::new(),
            features,
            resolution: None,
            demo: false,
            environment: BTreeMap::new(),
            launcher_brand: "RMCL".to_owned(),
            launcher_version: env!("CARGO_PKG_VERSION").to_owned(),
        })
    }

    pub fn from_install_plan(
        layout: &MinecraftLayout,
        java: JavaRuntime,
        version: &GameVersion,
        account: LaunchAccount,
        platform: &Platform,
        plan: &InstallPlan,
    ) -> Result<Self, LaunchError> {
        let mut options = Self::from_layout(layout, java, version, account, platform)?;
        options.libraries = plan
            .files
            .iter()
            .filter(|file| file.kind == PlannedFileKind::Library)
            .map(|file| absolute_path(&file.destination))
            .collect::<Result<_, _>>()?;
        Ok(options)
    }

    pub fn with_resolution(mut self, width: u32, height: u32) -> Self {
        self.resolution = Some((width, height));
        self.features
            .insert("has_custom_resolution".to_owned(), true);
        self
    }

    pub fn with_demo(mut self, demo: bool) -> Self {
        self.demo = demo;
        self.features.insert("is_demo_user".to_owned(), demo);
        self
    }
}

pub(crate) fn absolute_path(path: &Path) -> Result<PathBuf, LaunchError> {
    std::path::absolute(path).map_err(|source| LaunchError::AbsolutePath {
        path: path.display().to_string(),
        source,
    })
}
