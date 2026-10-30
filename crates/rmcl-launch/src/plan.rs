use std::{
    collections::BTreeMap,
    ffi::OsString,
    fmt,
    path::{Path, PathBuf},
};

use rmcl_domain::OperatingSystem;
use rmcl_java::JavaRequirement;
use rmcl_minecraft::evaluate_arguments;

use crate::{LaunchError, LaunchOptions, options::absolute_path, process::LogRedactor};

#[derive(Clone)]
pub struct LaunchPlan {
    pub java: PathBuf,
    pub main_class: String,
    pub jvm_arguments: Vec<String>,
    pub game_arguments: Vec<String>,
    pub classpath: Vec<PathBuf>,
    pub working_directory: PathBuf,
    pub natives_directory: PathBuf,
    pub environment: BTreeMap<String, String>,
    redaction_token: Option<String>,
}

impl fmt::Debug for LaunchPlan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let redactor = self.redactor();
        let redact = |arguments: &[String]| {
            arguments
                .iter()
                .map(|argument| redactor.redact(argument))
                .collect::<Vec<_>>()
        };
        let environment = self
            .environment
            .iter()
            .map(|(key, value)| (key.clone(), redactor.redact(value)))
            .collect::<BTreeMap<_, _>>();
        formatter
            .debug_struct("LaunchPlan")
            .field("java", &self.java)
            .field("main_class", &self.main_class)
            .field("jvm_arguments", &redact(&self.jvm_arguments))
            .field("game_arguments", &redact(&self.game_arguments))
            .field("classpath", &self.classpath)
            .field("working_directory", &self.working_directory)
            .field("natives_directory", &self.natives_directory)
            .field("environment", &environment)
            .finish_non_exhaustive()
    }
}

impl LaunchPlan {
    pub fn from_options(
        version: &rmcl_minecraft::GameVersion,
        mut options: LaunchOptions,
    ) -> Result<Self, LaunchError> {
        make_paths_absolute(&mut options)?;
        validate_launch_paths(&options)?;
        let main_class = version
            .main_class
            .clone()
            .ok_or_else(|| LaunchError::MissingMainClass(version.id.clone()))?;
        let requirement = JavaRequirement::for_version(version, &options.platform);
        if !requirement.accepts(&options.java) {
            return Err(LaunchError::IncompatibleJava {
                path: options.java.executable.display().to_string(),
                reason: format!("requirement {requirement:?}"),
            });
        }

        let classpath = std::env::join_paths(
            options
                .libraries
                .iter()
                .chain(std::iter::once(&options.client_jar)),
        )
        .map_err(|error| LaunchError::InvalidClasspath(error.to_string()))?;
        let classpath = classpath
            .into_string()
            .map_err(|path| LaunchError::NonUtf8Path(path.to_string_lossy().into_owned()))?;
        let placeholders = placeholders(version, &options, &classpath)?;
        let mut jvm_arguments = supporting_jvm_arguments(&options);
        if let Some(authlib) = &options.authlib {
            jvm_arguments.extend(authlib_jvm_arguments(authlib)?);
        }
        jvm_arguments.extend(match &version.arguments {
            Some(arguments) => {
                evaluate_arguments(&arguments.jvm, &options.platform, &options.features)?
                    .into_iter()
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            }
            None => default_jvm_arguments(&options),
        });
        let mut game_arguments = Vec::new();
        if let Some(legacy) = &version.minecraft_arguments {
            game_arguments.extend(crate::tokenize_legacy_arguments(legacy)?);
        }
        if let Some(arguments) = &version.arguments {
            game_arguments.extend(
                evaluate_arguments(&arguments.game, &options.platform, &options.features)?
                    .into_iter()
                    .map(str::to_owned),
            );
        }
        if version.minecraft_arguments.is_some()
            && let Some((width, height)) = options.resolution
        {
            game_arguments.extend([
                "--width".to_owned(),
                width.to_string(),
                "--height".to_owned(),
                height.to_string(),
            ]);
        }
        game_arguments.extend(options.additional_game_arguments);

        let jvm_arguments = jvm_arguments
            .into_iter()
            .chain(options.additional_jvm_arguments)
            .map(|argument| replace_placeholders(&argument, &placeholders))
            .collect();
        let mut game_arguments: Vec<String> = game_arguments
            .into_iter()
            .map(|argument| replace_placeholders(&argument, &placeholders))
            .collect();
        if let Some(server) = options.server.as_ref() {
            if supports_quick_play(&options.minecraft_version)? {
                game_arguments.push("--quickPlayMultiplayer".to_owned());
                game_arguments.push(server.quick_play_address());
            } else {
                game_arguments.push("--server".to_owned());
                game_arguments.push(server.legacy_host().to_owned());
                game_arguments.push("--port".to_owned());
                game_arguments.push(server.port().to_string());
            }
        } else if let Some(world) = options.quick_play_world.as_ref() {
            if !supports_quick_play(&options.minecraft_version)? {
                return Err(LaunchError::InvalidArguments(format!(
                    "Minecraft version `{}` does not support Quick Play singleplayer",
                    options.minecraft_version
                )));
            }
            game_arguments.push("--quickPlaySingleplayer".to_owned());
            game_arguments.push(world.clone());
        }
        Ok(Self {
            java: options.java.executable,
            main_class,
            jvm_arguments,
            game_arguments,
            classpath: options
                .libraries
                .into_iter()
                .chain(std::iter::once(options.client_jar))
                .collect(),
            working_directory: options.game_directory,
            natives_directory: options.natives_directory,
            environment: options.environment,
            redaction_token: Some(options.account.access_token),
        })
    }

    /// Returns the real process arguments, including secrets; do not use this for diagnostics.
    pub fn command_line(&self) -> Vec<OsString> {
        let mut command =
            Vec::with_capacity(self.jvm_arguments.len() + self.game_arguments.len() + 2);
        command.push(self.java.clone().into_os_string());
        command.extend(self.jvm_arguments.iter().map(OsString::from));
        command.push(OsString::from(self.main_class.clone()));
        command.extend(self.game_arguments.iter().map(OsString::from));
        command
    }

    /// Returns display arguments with the account access token replaced.
    pub fn diagnostic_command_line(&self) -> Vec<String> {
        let redactor = self.redactor();
        self.command_line()
            .into_iter()
            .map(|argument| redactor.redact(&argument.to_string_lossy()))
            .collect()
    }

    pub fn redactor(&self) -> LogRedactor {
        LogRedactor::new(self.redaction_token.clone())
    }
}

fn make_paths_absolute(options: &mut LaunchOptions) -> Result<(), LaunchError> {
    options.game_directory = absolute_path(&options.game_directory)?;
    options.library_directory = absolute_path(&options.library_directory)?;
    options.assets_root = absolute_path(&options.assets_root)?;
    options.natives_directory = absolute_path(&options.natives_directory)?;
    options.client_jar = absolute_path(&options.client_jar)?;
    for library in &mut options.libraries {
        *library = absolute_path(library)?;
    }
    if let Some(authlib) = &mut options.authlib {
        authlib.jar_path = absolute_path(&authlib.jar_path)?;
    }
    Ok(())
}

fn authlib_jvm_arguments(config: &crate::AuthlibLaunchConfig) -> Result<Vec<String>, LaunchError> {
    if config.api_root.trim().is_empty() {
        return Err(LaunchError::AuthlibApiRootBlank);
    }
    let path = utf8_path(&config.jar_path)?;
    if path.contains('=') {
        return Err(LaunchError::AuthlibInjectorPathContainsEquals {
            path: path.to_owned(),
        });
    }
    if !config.jar_path.is_file() {
        return Err(LaunchError::AuthlibInjectorMissing {
            path: path.to_owned(),
        });
    }
    let mut arguments = vec![
        format!("-javaagent:{path}={}", config.api_root.trim()),
        "-Dauthlibinjector.side=client".to_owned(),
    ];
    if let Some(prefetched) = config.prefetched_metadata.as_deref()
        && !prefetched.is_empty()
    {
        arguments.push(format!(
            "-Dauthlibinjector.yggdrasil.prefetched={prefetched}"
        ));
    }
    Ok(arguments)
}

fn validate_launch_paths(options: &LaunchOptions) -> Result<(), LaunchError> {
    for path in [
        &options.java.executable,
        &options.game_directory,
        &options.library_directory,
        &options.assets_root,
        &options.natives_directory,
        &options.client_jar,
    ] {
        utf8_path(path)?;
    }
    for library in &options.libraries {
        utf8_path(library)?;
    }
    Ok(())
}

fn utf8_path(path: &Path) -> Result<&str, LaunchError> {
    path.to_str()
        .ok_or_else(|| LaunchError::NonUtf8Path(path.to_string_lossy().into_owned()))
}

fn path_string(path: &Path) -> Result<String, LaunchError> {
    Ok(utf8_path(path)?.to_owned())
}

fn supporting_jvm_arguments(options: &LaunchOptions) -> Vec<String> {
    let mut arguments = vec![
        "-Dminecraft.client.jar=${primary_jar}".to_owned(),
        "-Djava.rmi.server.useCodebaseOnly=true".to_owned(),
        "-Dcom.sun.jndi.rmi.object.trustURLCodebase=false".to_owned(),
        "-Dcom.sun.jndi.cosnaming.object.trustURLCodebase=false".to_owned(),
        "-Dlog4j2.formatMsgNoLookups=true".to_owned(),
    ];
    if options.platform.os != OperatingSystem::Windows {
        arguments.push("-Duser.home=${game_directory_parent}".to_owned());
    }
    arguments
}

fn default_jvm_arguments(options: &LaunchOptions) -> Vec<String> {
    let mut arguments = Vec::new();
    if options.platform.os == OperatingSystem::Windows {
        arguments.push(
            "-XX:HeapDumpPath=MojangTricksIntelDriversForPerformance_javaw.exe_minecraft.exe.heapdump"
                .to_owned(),
        );
        if options.platform.os_version.starts_with("10.") {
            arguments.extend([
                "-Dos.name=Windows 10".to_owned(),
                "-Dos.version=10.0".to_owned(),
            ]);
        }
    }
    arguments.extend([
        "-Djava.library.path=${natives_directory}".to_owned(),
        "-Dminecraft.launcher.brand=${launcher_name}".to_owned(),
        "-Dminecraft.launcher.version=${launcher_version}".to_owned(),
        "-cp".to_owned(),
        "${classpath}".to_owned(),
    ]);
    arguments
}

fn placeholders(
    version: &rmcl_minecraft::GameVersion,
    options: &LaunchOptions,
    classpath: &str,
) -> Result<BTreeMap<String, String>, LaunchError> {
    let mut values = BTreeMap::from([
        (
            "auth_player_name".to_owned(),
            options.account.username.clone(),
        ),
        (
            "auth_uuid".to_owned(),
            options.account.uuid.simple().to_string(),
        ),
        (
            "auth_access_token".to_owned(),
            options.account.access_token.clone(),
        ),
        (
            "auth_session".to_owned(),
            options.account.access_token.clone(),
        ),
        ("user_type".to_owned(), options.account.user_type.clone()),
        (
            "user_properties".to_owned(),
            options.account.user_properties.clone(),
        ),
        ("version_name".to_owned(), version.id.clone()),
        (
            "game_directory".to_owned(),
            path_string(&options.game_directory)?,
        ),
        ("assets_root".to_owned(), path_string(&options.assets_root)?),
        ("game_assets".to_owned(), path_string(&options.assets_root)?),
        (
            "library_directory".to_owned(),
            path_string(&options.library_directory)?,
        ),
        (
            "libraries_directory".to_owned(),
            path_string(&options.library_directory)?,
        ),
        ("primary_jar".to_owned(), path_string(&options.client_jar)?),
        (
            "primary_jar_name".to_owned(),
            options
                .client_jar
                .file_name()
                .map(|name| {
                    name.to_str().map(str::to_owned).ok_or_else(|| {
                        LaunchError::NonUtf8Path(name.to_string_lossy().into_owned())
                    })
                })
                .transpose()?
                .unwrap_or_default(),
        ),
        (
            "file_separator".to_owned(),
            std::path::MAIN_SEPARATOR.to_string(),
        ),
        (
            "classpath_separator".to_owned(),
            if cfg!(windows) {
                ";".to_owned()
            } else {
                ":".to_owned()
            },
        ),
        (
            "game_directory_parent".to_owned(),
            path_string(
                options
                    .game_directory
                    .parent()
                    .unwrap_or(&options.game_directory),
            )?,
        ),
        ("profile_name".to_owned(), "Minecraft".to_owned()),
        (
            "assets_index_name".to_owned(),
            version
                .asset_index
                .as_ref()
                .map(|index| index.id.clone())
                .or_else(|| version.assets.clone())
                .unwrap_or_else(|| "legacy".to_owned()),
        ),
        (
            "natives_directory".to_owned(),
            path_string(&options.natives_directory)?,
        ),
        ("classpath".to_owned(), classpath.to_owned()),
        ("launcher_name".to_owned(), options.launcher_brand.clone()),
        ("launcher_brand".to_owned(), options.launcher_brand.clone()),
        (
            "launcher_version".to_owned(),
            options.launcher_version.clone(),
        ),
        (
            "version_type".to_owned(),
            version
                .version_type
                .clone()
                .unwrap_or_else(|| "release".to_owned()),
        ),
    ]);
    if let Some((width, height)) = options.resolution {
        values.insert("resolution_width".to_owned(), width.to_string());
        values.insert("resolution_height".to_owned(), height.to_string());
    }
    Ok(values)
}

pub fn supports_quick_play(version: &str) -> Result<bool, LaunchError> {
    let version = version.trim();
    if let Some(supported) = release_version_supports_quick_play(version) {
        return Ok(supported);
    }
    if let Some(supported) = snapshot_supports_quick_play(version) {
        return Ok(supported);
    }
    if version.starts_with('a') || version.starts_with('b') {
        return Ok(false);
    }
    Err(LaunchError::InvalidArguments(format!(
        "cannot determine whether Minecraft version `{version}` supports Quick Play"
    )))
}

fn release_version_supports_quick_play(version: &str) -> Option<bool> {
    let base = version.split(['-', '+']).next()?;
    let mut parts = base.split('.');
    let major = parts.next()?.parse::<u32>().ok()?;
    if major == 1 {
        let minor = parts.next()?.parse::<u32>().ok()?;
        Some(minor >= 20)
    } else if major >= 20 {
        Some(true)
    } else {
        None
    }
}

fn snapshot_supports_quick_play(version: &str) -> Option<bool> {
    let (year, rest) = version.split_once('w')?;
    let year = year.parse::<u32>().ok()?;
    let week = rest
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse::<u32>()
        .ok()?;
    Some((year, week) >= (23, 14))
}

fn replace_placeholders(argument: &str, values: &BTreeMap<String, String>) -> String {
    let mut output = String::with_capacity(argument.len());
    let mut remaining = argument;
    while let Some(start) = remaining.find("${") {
        output.push_str(&remaining[..start]);
        let after_start = &remaining[start + 2..];
        let Some(end) = after_start.find('}') else {
            output.push_str(&remaining[start..]);
            return output;
        };
        let key = &after_start[..end];
        if let Some(value) = values.get(key) {
            output.push_str(value);
        } else {
            output.push_str("${");
            output.push_str(key);
            output.push('}');
        }
        remaining = &after_start[end + 1..];
    }
    output.push_str(remaining);
    output
}
