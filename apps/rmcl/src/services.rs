use std::{path::PathBuf, rc::Rc};

use rmcl_app::{AppError, JavaService, JavaServiceConfig, TaskManager, VanillaInstaller};
use rmcl_domain::{OperatingSystem, Platform};
use rmcl_net::DownloadConfig;
use tokio::runtime::{Builder, Runtime};

pub struct Services {
    pub runtime: Runtime,
    pub tasks: TaskManager,
    pub platform: Platform,
}

impl Services {
    pub fn new(java_store: PathBuf) -> Result<Rc<Self>, AppError> {
        let runtime = Builder::new_multi_thread()
            .enable_all()
            .thread_name("rmcl-core")
            .build()
            .map_err(|source| AppError::Io {
                operation: "create Tokio runtime",
                path: PathBuf::from("<runtime>"),
                source,
            })?;
        let installer = VanillaInstaller::official(DownloadConfig::default())?;
        let mut java_config = JavaServiceConfig::new(java_store);
        java_config.auto_install = true;
        let java = JavaService::new(java_config)?;
        let tasks = TaskManager::with_java(installer, java, 256);
        Ok(Rc::new(Self {
            runtime,
            tasks,
            platform: current_platform(),
        }))
    }
}

#[derive(Debug, Clone)]
pub struct Bootstrap {
    pub minecraft_root: PathBuf,
    pub java_store: PathBuf,
    pub version_id: String,
    pub username: String,
}

impl Bootstrap {
    pub fn from_args() -> Result<Option<Self>, String> {
        let home = home_directory();
        let mut bootstrap = Self {
            minecraft_root: home.join(".minecraft"),
            java_store: home.join(".rmcl").join("java"),
            version_id: "26.3".to_owned(),
            username: "Steve".to_owned(),
        };
        let mut arguments = std::env::args_os().skip(1);
        while let Some(argument) = arguments.next() {
            match argument.to_str() {
                Some("--help") | Some("-h") => return Ok(None),
                Some("--minecraft-root") => {
                    bootstrap.minecraft_root =
                        PathBuf::from(next_value(&mut arguments, "--minecraft-root")?);
                }
                Some("--java-store") => {
                    bootstrap.java_store =
                        PathBuf::from(next_value(&mut arguments, "--java-store")?);
                }
                Some("--version") => {
                    bootstrap.version_id =
                        next_value(&mut arguments, "--version")?
                            .into_string()
                            .map_err(|_| "--version must be valid UTF-8".to_owned())?;
                }
                Some("--player") => {
                    bootstrap.username = next_value(&mut arguments, "--player")?
                        .into_string()
                        .map_err(|_| "--player must be valid UTF-8".to_owned())?;
                }
                _ => {
                    return Err(format!(
                        "unsupported argument `{}`; use --help",
                        argument.to_string_lossy()
                    ));
                }
            }
        }
        Ok(Some(bootstrap))
    }

    pub fn usage() -> &'static str {
        "RMCL\n\nOptions:\n  --minecraft-root <PATH>  Minecraft repository (default: ~/.minecraft)\n  --java-store <PATH>      Managed Java store (default: ~/.rmcl/java)\n  --version <ID>           Vanilla version ID (default: 26.3)\n  --player <NAME>          Offline player name (default: Steve)\n  -h, --help               Show this help"
    }
}

fn next_value(
    arguments: &mut impl Iterator<Item = std::ffi::OsString>,
    option: &str,
) -> Result<std::ffi::OsString, String> {
    arguments
        .next()
        .ok_or_else(|| format!("{option} requires a value"))
}

fn home_directory() -> PathBuf {
    #[cfg(windows)]
    let variable = "USERPROFILE";
    #[cfg(not(windows))]
    let variable = "HOME";
    std::env::var_os(variable)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

fn current_platform() -> Platform {
    let os = if cfg!(windows) {
        OperatingSystem::Windows
    } else if cfg!(target_os = "macos") {
        OperatingSystem::MacOs
    } else {
        OperatingSystem::Linux
    };
    Platform::new(
        os,
        sysinfo::System::os_version().unwrap_or_default(),
        std::env::consts::ARCH,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_defaults_do_not_share_hmcl_configuration() {
        let bootstrap = Bootstrap {
            minecraft_root: PathBuf::from("game"),
            java_store: PathBuf::from("runtime"),
            version_id: "test".to_owned(),
            username: "Steve".to_owned(),
        };
        assert_eq!(bootstrap.java_store, PathBuf::from("runtime"));
        assert!(Bootstrap::usage().contains("--minecraft-root"));
    }
}
