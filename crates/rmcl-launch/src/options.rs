use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use rmcl_domain::{Platform, ServerAddress};
use rmcl_install::{InstallPlan, PlannedFileKind};
use rmcl_instance::{InstanceLayout, IsolationMode, MinecraftLayout, VersionId};
use rmcl_java::JavaRuntime;
use rmcl_minecraft::{AssetCatalog, FeatureMap, GameVersion};

use crate::{LaunchAccount, LaunchError};

#[derive(Debug, Clone)]
pub struct LaunchOptions {
    pub java: JavaRuntime,
    pub account: LaunchAccount,
    pub platform: Platform,
    pub game_directory: PathBuf,
    /// Shared Maven library root, independent of the instance game directory.
    pub library_directory: PathBuf,
    pub assets_root: PathBuf,
    pub natives_directory: PathBuf,
    pub client_jar: PathBuf,
    pub libraries: Vec<PathBuf>,
    pub additional_jvm_arguments: Vec<String>,
    pub additional_game_arguments: Vec<String>,
    /// Optional authlib-injector Java agent and prefetched metadata.
    pub authlib: Option<crate::AuthlibLaunchConfig>,
    /// Base Minecraft version used to choose Quick Play versus legacy
    /// `--server`/`--port` arguments. Callers must preserve the base version
    /// for loader instances.
    pub minecraft_version: String,
    pub server: Option<ServerAddress>,
    /// World folder name passed through `--quickPlaySingleplayer`.
    pub quick_play_world: Option<String>,
    pub features: FeatureMap,
    pub resolution: Option<(u32, u32)>,
    pub demo: bool,
    pub environment: BTreeMap<String, String>,
    pub launcher_brand: String,
    pub launcher_version: String,
}

impl LaunchOptions {
    /// Builds options for a legacy shared-directory launch where the version ID
    /// is also the instance directory name.
    pub fn from_layout(
        layout: &MinecraftLayout,
        java: JavaRuntime,
        version: &GameVersion,
        account: LaunchAccount,
        platform: &Platform,
    ) -> Result<Self, LaunchError> {
        let id = VersionId::new(version.id.clone())?;
        let instance = layout.instance(id, IsolationMode::Shared);
        Self::from_instance(&instance, java, version, account, platform)
    }

    /// Builds options for an explicit instance. The game directory follows the
    /// instance isolation mode, while libraries, assets and the client JAR stay
    /// in the shared repository.
    pub fn from_instance(
        instance: &InstanceLayout,
        java: JavaRuntime,
        version: &GameVersion,
        account: LaunchAccount,
        platform: &Platform,
    ) -> Result<Self, LaunchError> {
        let repository = MinecraftLayout::new(absolute_path(instance.repository().root())?);
        let assets_root = effective_assets_root(&repository, version);
        let instance = repository.instance(instance.id().clone(), instance.isolation());
        let jar_id = VersionId::new(version.jar.as_deref().unwrap_or(&version.id))?;
        let mut features = FeatureMap::new();
        features.insert("has_custom_resolution".to_owned(), false);
        features.insert("is_demo_user".to_owned(), false);
        Ok(Self {
            java,
            account,
            platform: platform.clone(),
            game_directory: instance.game_dir(),
            library_directory: instance.repository().libraries_dir(),
            assets_root,
            natives_directory: instance.natives_dir(platform),
            client_jar: instance.client_jar_path_for(Some(&jar_id)),
            libraries: Vec::new(),
            additional_jvm_arguments: Vec::new(),
            additional_game_arguments: Vec::new(),
            authlib: None,
            minecraft_version: version.id.clone(),
            server: None,
            quick_play_world: None,
            features,
            resolution: None,
            demo: false,
            environment: BTreeMap::new(),
            launcher_brand: "RMCL".to_owned(),
            launcher_version: env!("CARGO_PKG_VERSION").to_owned(),
        })
    }

    pub fn from_install_plan(
        instance: &InstanceLayout,
        java: JavaRuntime,
        version: &GameVersion,
        account: LaunchAccount,
        platform: &Platform,
        plan: &InstallPlan,
    ) -> Result<Self, LaunchError> {
        let mut options = Self::from_instance(instance, java, version, account, platform)?;
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

    pub fn with_server(
        mut self,
        server: ServerAddress,
        minecraft_version: impl Into<String>,
    ) -> Self {
        self.server = Some(server);
        self.minecraft_version = minecraft_version.into();
        self
    }

    pub fn with_world(mut self, world: impl Into<String>) -> Self {
        self.quick_play_world = Some(world.into());
        self
    }
}

/// Picks the directory supplied through `--assetsDir` for an instance.
///
/// Legacy `virtual` and `map_to_resources` indexes are materialized under
/// `assets/virtual/<index>` by the installer, so the game must receive that
/// flattened directory instead of the content-addressed `assets/` root.
fn effective_assets_root(repository: &MinecraftLayout, version: &GameVersion) -> PathBuf {
    let assets_root = repository.assets_dir();
    let Some(index) = version.asset_index.as_ref() else {
        return assets_root;
    };
    let Ok(index_path) = repository.asset_index_path(&index.id) else {
        return assets_root;
    };
    let Ok(json) = std::fs::read_to_string(index_path) else {
        return assets_root;
    };
    let Ok(catalog) = AssetCatalog::from_json(&json) else {
        return assets_root;
    };
    if catalog.is_virtual == Some(true) || catalog.map_to_resources == Some(true) {
        return repository
            .virtual_assets_dir(&index.id)
            .unwrap_or(assets_root);
    }
    assets_root
}

pub(crate) fn absolute_path(path: &Path) -> Result<PathBuf, LaunchError> {
    std::path::absolute(path).map_err(|source| LaunchError::AbsolutePath {
        path: path.display().to_string(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(asset_index: &str) -> GameVersion {
        GameVersion::from_json(&format!(
            r#"{{"id":"test","assetIndex":{{"id":"{asset_index}","url":"https://example.invalid/index.json"}}}}"#
        ))
        .unwrap()
    }

    #[test]
    fn legacy_asset_index_uses_the_materialized_virtual_directory() {
        let temporary = tempfile::tempdir().unwrap();
        let layout = MinecraftLayout::new(temporary.path());
        let index_path = layout.asset_index_path("pre-1.6").unwrap();
        std::fs::create_dir_all(index_path.parent().unwrap()).unwrap();
        std::fs::write(&index_path, r#"{"map_to_resources":true,"objects":{}}"#).unwrap();

        assert_eq!(
            effective_assets_root(&layout, &version("pre-1.6")),
            layout.virtual_assets_dir("pre-1.6").unwrap()
        );
    }

    #[test]
    fn modern_asset_index_uses_the_shared_asset_root() {
        let temporary = tempfile::tempdir().unwrap();
        let layout = MinecraftLayout::new(temporary.path());
        let index_path = layout.asset_index_path("17").unwrap();
        std::fs::create_dir_all(index_path.parent().unwrap()).unwrap();
        std::fs::write(&index_path, r#"{"objects":{}}"#).unwrap();

        assert_eq!(
            effective_assets_root(&layout, &version("17")),
            layout.assets_dir()
        );
    }
}
