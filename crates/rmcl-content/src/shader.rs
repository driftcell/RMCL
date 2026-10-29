//! Shader-pack selection stored by Iris, Oculus, or OptiFine.

use std::{fs, path::PathBuf};

use rmcl_instance::InstanceLayout;

use crate::{ContentError, ShaderLoader, properties::PropertiesFile};

/// The selected shader pack and whether shader rendering is enabled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShaderPackSelection {
    /// File or folder name relative to `shaderpacks/`.
    pub active: Option<String>,
    pub enabled: bool,
}

/// Reads the shader-pack selection for a supported loader.
///
/// Returns `None` when the loader has not created its configuration yet.
/// Iris and Oculus keep a selected pack while `enableShaders=false`; OptiFine
/// clears `shaderPack` to disable shaders.
pub fn read_shader_pack_selection(
    instance: &InstanceLayout,
    loader: ShaderLoader,
) -> Result<Option<ShaderPackSelection>, ContentError> {
    let path = shader_config_path(instance, loader);
    let Some(file) = PropertiesFile::load(path)? else {
        return Ok(None);
    };
    let active = file
        .get("shaderPack")
        .filter(|pack| !pack.trim().is_empty());
    let enabled = match loader {
        ShaderLoader::Iris | ShaderLoader::Oculus => file
            .get("enableShaders")
            .and_then(|value| parse_bool(&value))
            .unwrap_or(active.is_some()),
        ShaderLoader::OptiFine => active.is_some(),
    };
    Ok(Some(ShaderPackSelection { active, enabled }))
}

/// Selects or disables a shader pack.
///
/// `Some(pack)` must name an existing file or folder directly under the
/// instance's `shaderpacks/` directory. `None` disables shader rendering while
/// preserving the selected Iris/Oculus pack; OptiFine has no separate enable
/// flag, so its selection is cleared.
pub fn set_shader_pack_selection(
    instance: &InstanceLayout,
    loader: ShaderLoader,
    pack: Option<&str>,
) -> Result<(), ContentError> {
    let path = shader_config_path(instance, loader);
    let mut file = PropertiesFile::load(path.clone())?.unwrap_or_else(|| PropertiesFile::new(path));
    match pack {
        Some(pack) => {
            validate_shader_pack(instance, pack)?;
            file.set("shaderPack", pack);
            if matches!(loader, ShaderLoader::Iris | ShaderLoader::Oculus) {
                file.set("enableShaders", "true");
            }
        }
        None => match loader {
            ShaderLoader::Iris | ShaderLoader::Oculus => {
                file.set("enableShaders", "false");
            }
            ShaderLoader::OptiFine => {
                file.set("shaderPack", "");
            }
        },
    }
    file.save()
}

fn shader_config_path(instance: &InstanceLayout, loader: ShaderLoader) -> PathBuf {
    match loader {
        ShaderLoader::Iris => instance.game_dir().join("config/iris.properties"),
        ShaderLoader::Oculus => instance.game_dir().join("config/oculus.properties"),
        ShaderLoader::OptiFine => instance.game_dir().join("optionsshaders.txt"),
    }
}

fn validate_shader_pack(instance: &InstanceLayout, pack: &str) -> Result<(), ContentError> {
    let directory = instance.shaderpacks_dir();
    let path = directory.join(pack);
    if pack.trim().is_empty() || pack == "." || pack == ".." || pack.contains(['/', '\\', '\0']) {
        return Err(ContentError::UnsafePath {
            path,
            reason: "shader pack must be a single file or folder name".to_owned(),
        });
    }
    let metadata = fs::symlink_metadata(&path).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            ContentError::NotFound { path: path.clone() }
        } else {
            ContentError::Io {
                operation: "inspect shader pack selection",
                path: path.clone(),
                source,
            }
        }
    })?;
    if metadata.file_type().is_symlink() {
        return Err(ContentError::Symlink { path });
    }
    if !metadata.is_file() && !metadata.is_dir() {
        return Err(ContentError::NotFound { path });
    }
    Ok(())
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use rmcl_instance::{IsolationMode, MinecraftLayout, VersionId};
    use tempfile::TempDir;

    use super::*;

    fn instance(root: &Path) -> InstanceLayout {
        let instance = MinecraftLayout::new(root)
            .instance(VersionId::new("1.21.1").unwrap(), IsolationMode::Isolated);
        instance.ensure_content_dirs().unwrap();
        instance
    }

    fn write(path: &Path, contents: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn selects_disables_and_keeps_iris_pack() {
        let temporary = TempDir::new().unwrap();
        let instance = instance(temporary.path());
        write(&instance.shaderpacks_dir().join("Nice Pack.zip"), b"shader");

        assert_eq!(
            read_shader_pack_selection(&instance, ShaderLoader::Iris).unwrap(),
            None
        );

        set_shader_pack_selection(&instance, ShaderLoader::Iris, Some("Nice Pack.zip")).unwrap();
        let selection = read_shader_pack_selection(&instance, ShaderLoader::Iris)
            .unwrap()
            .unwrap();
        assert_eq!(selection.active.as_deref(), Some("Nice Pack.zip"));
        assert!(selection.enabled);

        set_shader_pack_selection(&instance, ShaderLoader::Iris, None).unwrap();
        let selection = read_shader_pack_selection(&instance, ShaderLoader::Iris)
            .unwrap()
            .unwrap();
        assert_eq!(selection.active.as_deref(), Some("Nice Pack.zip"));
        assert!(!selection.enabled);

        let config =
            fs::read_to_string(instance.game_dir().join("config/iris.properties")).unwrap();
        assert!(config.contains("shaderPack=Nice Pack.zip"));
        assert!(config.contains("enableShaders=false"));
    }

    #[test]
    fn oculus_uses_its_own_configuration() {
        let temporary = TempDir::new().unwrap();
        let instance = instance(temporary.path());
        write(&instance.shaderpacks_dir().join("shader.zip"), b"shader");

        set_shader_pack_selection(&instance, ShaderLoader::Oculus, Some("shader.zip")).unwrap();
        assert!(
            instance
                .game_dir()
                .join("config/oculus.properties")
                .is_file()
        );
        assert!(
            read_shader_pack_selection(&instance, ShaderLoader::Oculus)
                .unwrap()
                .unwrap()
                .enabled
        );
    }

    #[test]
    fn optifine_clears_selection_when_disabled() {
        let temporary = TempDir::new().unwrap();
        let instance = instance(temporary.path());
        write(&instance.shaderpacks_dir().join("shader.zip"), b"shader");
        write(
            &instance.game_dir().join("optionsshaders.txt"),
            b"# keep\nshaderPack=other.zip\n",
        );

        set_shader_pack_selection(&instance, ShaderLoader::OptiFine, Some("shader.zip")).unwrap();
        let selection = read_shader_pack_selection(&instance, ShaderLoader::OptiFine)
            .unwrap()
            .unwrap();
        assert_eq!(selection.active.as_deref(), Some("shader.zip"));
        assert!(selection.enabled);
        let config = fs::read_to_string(instance.game_dir().join("optionsshaders.txt")).unwrap();
        assert!(config.contains("# keep"));

        set_shader_pack_selection(&instance, ShaderLoader::OptiFine, None).unwrap();
        let selection = read_shader_pack_selection(&instance, ShaderLoader::OptiFine)
            .unwrap()
            .unwrap();
        assert_eq!(selection.active, None);
        assert!(!selection.enabled);
    }

    #[test]
    fn rejects_unsafe_or_missing_pack_names() {
        let temporary = TempDir::new().unwrap();
        let instance = instance(temporary.path());
        write(&instance.shaderpacks_dir().join("shader.zip"), b"shader");

        assert!(matches!(
            set_shader_pack_selection(&instance, ShaderLoader::Iris, Some("../shader.zip")),
            Err(ContentError::UnsafePath { .. })
        ));
        assert!(matches!(
            set_shader_pack_selection(&instance, ShaderLoader::Iris, Some("missing.zip")),
            Err(ContentError::NotFound { .. })
        ));
    }
}
