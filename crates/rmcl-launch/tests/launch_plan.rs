use std::path::PathBuf;

use rmcl_auth::{AuthlibAccount, OfflineAccount};
use rmcl_domain::{Architecture, OperatingSystem, Platform, ServerAddress};
use rmcl_install::{InstallPlan, PlannedFile, PlannedFileKind};
use rmcl_instance::{IsolationMode, MinecraftLayout, VersionId};
use rmcl_java::{JavaRuntime, JavaRuntimeSource};
use rmcl_launch::{AuthlibLaunchConfig, LaunchAccount, LaunchError, LaunchOptions, LaunchPlan};
use rmcl_minecraft::GameVersion;
use url::Url;
use uuid::Uuid;

fn platform() -> Platform {
    Platform::new(OperatingSystem::Linux, "6.1", "x86_64")
}

fn java() -> JavaRuntime {
    JavaRuntime {
        executable: PathBuf::from("java"),
        version: "21.0.7".to_owned(),
        major_version: 21,
        operating_system: OperatingSystem::Linux,
        architecture: Architecture::X86_64,
        vendor: None,
        source: JavaRuntimeSource::Managed,
    }
}

fn absolute(path: &str) -> PathBuf {
    std::path::absolute(path).unwrap()
}

fn native_argument(version: &str) -> String {
    format!(
        "-Djava.library.path={}",
        absolute(&format!(
            ".minecraft/versions/{version}/natives-linux-x86_64"
        ))
        .display()
    )
}

fn account() -> LaunchAccount {
    LaunchAccount {
        username: "Steve".to_owned(),
        uuid: Uuid::parse_str("5627dd98-e6be-3c21-b8a8-e92344183641").unwrap(),
        access_token: "secret-token".to_owned(),
        user_type: "msa".to_owned(),
        user_properties: "{}".to_owned(),
    }
}

fn modern_version() -> GameVersion {
    GameVersion::from_json(
        r#"{
            "id": "test",
            "mainClass": "example.Main",
            "javaVersion": {"component": "java-runtime-delta", "majorVersion": 21},
            "assetIndex": {"id": "17", "url": "https://example.invalid/index.json"},
            "arguments": {
                "jvm": ["-Djava.library.path=${natives_directory}", "-cp", "${classpath}"],
                "game": [
                    "--username", "${auth_player_name}",
                    "--accessToken", "${auth_access_token}",
                    "--demo",
                    "${unknown}"
                ]
            }
        }"#,
    )
    .unwrap()
}

fn options() -> LaunchOptions {
    let version = modern_version();
    let mut options = LaunchOptions::from_layout(
        &MinecraftLayout::new(".minecraft"),
        java(),
        &version,
        account(),
        &platform(),
    )
    .unwrap();
    options.libraries = vec![PathBuf::from("lib/a.jar"), PathBuf::from("lib/b.jar")];
    options.additional_game_arguments = vec!["--custom".to_owned()];
    options.with_demo(true)
}

#[cfg(unix)]
#[test]
fn rejects_non_utf8_launch_paths() {
    use std::os::unix::ffi::OsStringExt;

    let mut options = options();
    options.game_directory = PathBuf::from(std::ffi::OsString::from_vec(b"bad-\xff".to_vec()));

    assert!(matches!(
        LaunchPlan::from_options(&modern_version(), options),
        Err(LaunchError::NonUtf8Path(_))
    ));
}

#[test]
fn builds_modern_launch_plan_with_safe_diagnostics() {
    let version = modern_version();
    let options = options();

    let plan = LaunchPlan::from_options(&version, options).unwrap();

    let command = plan.command_line();
    assert_eq!(command[plan.jvm_arguments.len() + 1], "example.Main");
    assert!(
        plan.jvm_arguments
            .iter()
            .any(|argument| argument == &native_argument("test"))
    );
    let client_jar = absolute(".minecraft/versions/test/test.jar");
    assert!(
        plan.jvm_arguments
            .iter()
            .any(|argument| argument == &format!("-Dminecraft.client.jar={}", client_jar.display()))
    );
    assert!(
        plan.jvm_arguments
            .iter()
            .any(|argument| argument == "-Djava.rmi.server.useCodebaseOnly=true")
    );
    assert!(
        plan.jvm_arguments
            .iter()
            .any(|argument| argument.starts_with("-Duser.home="))
    );

    let classpath = std::env::join_paths([
        absolute("lib/a.jar"),
        absolute("lib/b.jar"),
        client_jar.clone(),
    ])
    .unwrap();
    let classpath = classpath.to_string_lossy();
    let classpath_index = plan
        .jvm_arguments
        .iter()
        .position(|argument| argument == "-cp")
        .unwrap();
    assert_eq!(plan.jvm_arguments[classpath_index + 1], classpath);
    assert_eq!(plan.classpath.last().unwrap(), &client_jar);
    assert!(plan.classpath.iter().all(|path| path.is_absolute()));
    assert!(plan.working_directory.is_absolute());
    assert!(plan.natives_directory.is_absolute());

    assert_eq!(
        plan.game_arguments,
        [
            "--username",
            "Steve",
            "--accessToken",
            "secret-token",
            "--demo",
            "${unknown}",
            "--custom"
        ]
    );
    let diagnostic = plan.diagnostic_command_line().join(" ");
    assert!(!diagnostic.contains("secret-token"));
    assert!(diagnostic.contains("<redacted>"));
}

#[test]
fn isolated_instance_uses_its_own_game_dir_and_shared_libraries() {
    let version = GameVersion::from_json(
        r#"{
            "id": "fabric-1.21.1",
            "mainClass": "example.Main",
            "javaVersion": {"component": "java-runtime-delta", "majorVersion": 21},
            "arguments": {
                "jvm": ["-Dlib=${library_directory}", "-cp", "${classpath}"],
                "game": ["--gameDir", "${game_directory}"]
            }
        }"#,
    )
    .unwrap();
    let repository = MinecraftLayout::new(".minecraft");
    let instance = repository.instance(
        VersionId::new("fabric-1.21.1").unwrap(),
        IsolationMode::Isolated,
    );

    let options =
        LaunchOptions::from_instance(&instance, java(), &version, account(), &platform()).unwrap();
    let plan = LaunchPlan::from_options(&version, options).unwrap();

    assert_eq!(
        plan.working_directory,
        absolute(".minecraft/versions/fabric-1.21.1")
    );
    assert_eq!(
        plan.natives_directory,
        absolute(".minecraft/versions/fabric-1.21.1/natives-linux-x86_64")
    );
    assert!(plan.jvm_arguments.iter().any(|argument| {
        argument == &format!("-Dlib={}", absolute(".minecraft/libraries").display())
    }));
    let game_dir = absolute(".minecraft/versions/fabric-1.21.1")
        .display()
        .to_string();
    assert_eq!(plan.game_arguments, ["--gameDir", game_dir.as_str()]);
}

#[test]
fn instance_client_jar_follows_the_version_jar_redirect() {
    let version = GameVersion::from_json(
        r#"{
            "id": "fabric-1.21.1",
            "jar": "1.21.1",
            "mainClass": "example.Main",
            "javaVersion": {"component": "java-runtime-delta", "majorVersion": 21}
        }"#,
    )
    .unwrap();
    let repository = MinecraftLayout::new(".minecraft");
    let instance = repository.instance(
        VersionId::new("fabric-1.21.1").unwrap(),
        IsolationMode::Isolated,
    );

    let options =
        LaunchOptions::from_instance(&instance, java(), &version, account(), &platform()).unwrap();

    assert_eq!(
        options.client_jar,
        absolute(".minecraft/versions/1.21.1/1.21.1.jar")
    );
}

#[test]
fn debug_formatting_redacts_account_and_process_secrets() {
    let version = modern_version();
    let options = options();
    let options_debug = format!("{options:?}");
    let plan = LaunchPlan::from_options(&version, options).unwrap();
    let plan_debug = format!("{plan:?}");
    let redactor_debug = format!("{:?}", plan.redactor());

    for diagnostic in [options_debug, plan_debug, redactor_debug] {
        assert!(!diagnostic.contains("secret-token"), "{diagnostic}");
    }
}

#[test]
fn empty_jvm_does_not_add_default_arguments() {
    let mut version = modern_version();
    version.arguments = Some(rmcl_minecraft::Arguments {
        game: Vec::new(),
        jvm: Vec::new(),
    });
    let options = options();

    let plan = LaunchPlan::from_options(&version, options).unwrap();

    assert!(!plan.jvm_arguments.iter().any(|argument| argument == "-cp"));
    assert!(
        !plan
            .jvm_arguments
            .iter()
            .any(|argument| argument == &native_argument("test"))
    );
    assert!(
        plan.jvm_arguments
            .iter()
            .any(|argument| argument.starts_with("-Dminecraft.client.jar="))
    );
}

#[test]
fn legacy_arguments_receive_defaults_and_resolution() {
    let version = GameVersion::from_json(
        r#"{
            "id": "legacy",
            "mainClass": "example.Main",
            "minecraftArguments": "--username ${auth_player_name} --accessToken ${auth_access_token} --custom ${unknown}",
            "javaVersion": {"component": "jre-legacy", "majorVersion": 8}
        }"#,
    )
    .unwrap();
    let mut options = LaunchOptions::from_layout(
        &MinecraftLayout::new(".minecraft"),
        java(),
        &version,
        account(),
        &platform(),
    )
    .unwrap();
    options = options.with_resolution(1024, 768);

    let plan = LaunchPlan::from_options(&version, options).unwrap();

    assert!(
        plan.jvm_arguments
            .iter()
            .any(|argument| argument == &native_argument("legacy"))
    );
    assert!(
        plan.jvm_arguments
            .iter()
            .any(|argument| argument == "-Dminecraft.launcher.brand=RMCL")
    );
    assert!(plan.jvm_arguments.iter().any(|argument| argument == "-cp"));
    assert!(
        plan.game_arguments
            .windows(2)
            .any(|pair| pair == ["--width", "1024"])
    );
    assert!(
        plan.game_arguments
            .windows(2)
            .any(|pair| pair == ["--height", "768"])
    );
    assert!(plan.game_arguments.contains(&"${unknown}".to_owned()));
    assert!(plan.game_arguments.contains(&"secret-token".to_owned()));
}

#[test]
fn windows_defaults_include_heap_and_os_overrides() {
    let version = GameVersion::from_json(
        r#"{"id":"legacy","mainClass":"example.Main","minecraftArguments":"--demo"}"#,
    )
    .unwrap();
    let platform = Platform::new(OperatingSystem::Windows, "10.0.22631", "x86_64");
    let mut windows_java = java();
    windows_java.operating_system = OperatingSystem::Windows;
    let options = LaunchOptions::from_layout(
        &MinecraftLayout::new(".minecraft"),
        windows_java,
        &version,
        account(),
        &platform,
    )
    .unwrap();

    let plan = LaunchPlan::from_options(&version, options).unwrap();

    assert!(
        plan.jvm_arguments
            .iter()
            .any(|argument| argument.starts_with("-XX:HeapDumpPath="))
    );
    assert!(
        plan.jvm_arguments
            .iter()
            .any(|argument| argument == "-Dos.name=Windows 10")
    );
    assert!(
        plan.jvm_arguments
            .iter()
            .any(|argument| argument == "-Dos.version=10.0")
    );
    assert!(
        !plan
            .jvm_arguments
            .iter()
            .any(|argument| argument.starts_with("-Duser.home="))
    );
}

#[test]
fn rejects_incompatible_java_and_missing_main_class() {
    let version = modern_version();
    let mut old_java = java();
    old_java.major_version = 8;
    let options = LaunchOptions::from_layout(
        &MinecraftLayout::new(".minecraft"),
        old_java,
        &version,
        account(),
        &platform(),
    )
    .unwrap();
    assert!(matches!(
        LaunchPlan::from_options(&version, options),
        Err(LaunchError::IncompatibleJava { .. })
    ));

    let version = GameVersion::from_json(r#"{"id":"empty"}"#).unwrap();
    let options = LaunchOptions::from_layout(
        &MinecraftLayout::new(".minecraft"),
        java(),
        &version,
        account(),
        &platform(),
    )
    .unwrap();
    assert!(matches!(
        LaunchPlan::from_options(&version, options),
        Err(LaunchError::MissingMainClass(_))
    ));
}

#[test]
fn install_plan_only_contributes_classpath_libraries() {
    let version = modern_version();
    let plan = InstallPlan {
        files: vec![
            PlannedFile {
                kind: PlannedFileKind::Library,
                destination: PathBuf::from("libraries/a.jar"),
                sources: vec![Url::parse("https://example.invalid/a.jar").unwrap()],
                sha1: None,
                size: None,
            },
            PlannedFile {
                kind: PlannedFileKind::NativeLibrary,
                destination: PathBuf::from("libraries/native.jar"),
                sources: vec![Url::parse("https://example.invalid/native.jar").unwrap()],
                sha1: None,
                size: None,
            },
            PlannedFile {
                kind: PlannedFileKind::Client,
                destination: PathBuf::from("versions/test/test.jar"),
                sources: vec![Url::parse("https://example.invalid/test.jar").unwrap()],
                sha1: None,
                size: None,
            },
        ],
        extractions: Vec::new(),
    };
    let instance = MinecraftLayout::new(".minecraft")
        .instance(VersionId::new("test").unwrap(), IsolationMode::Shared);
    let options = LaunchOptions::from_install_plan(
        &instance,
        java(),
        &version,
        account(),
        &platform(),
        &plan,
    )
    .unwrap();

    assert_eq!(options.libraries, [absolute("libraries/a.jar")]);
}

#[test]
fn offline_account_factory_produces_launch_identity() {
    let offline = OfflineAccount::new("Steve").unwrap();
    let account = LaunchAccount::from_offline(&offline);

    assert_eq!(account.username, "Steve");
    assert_eq!(account.uuid, offline.uuid());
    assert_eq!(account.access_token.len(), 32);
    assert_eq!(account.user_type, "msa");
    assert_eq!(account.user_properties, "{}");
}

#[test]
fn microsoft_account_factory_preserves_token_and_profile() {
    let uuid = uuid::Uuid::parse_str("986dec87b7ec47ff89ff033fdb95c4b5").unwrap();
    let profile = rmcl_auth::MicrosoftAccount::new("HowDoesAuthWork", uuid).unwrap();
    let account = LaunchAccount::from_microsoft(&profile, "minecraft-access");

    assert_eq!(account.username, "HowDoesAuthWork");
    assert_eq!(account.uuid, uuid);
    assert_eq!(account.access_token, "minecraft-access");
    assert_eq!(account.user_type, "msa");
    assert_eq!(account.user_properties, "{}");
    assert!(!format!("{account:?}").contains("minecraft-access"));
}

#[test]
fn authlib_account_factory_preserves_token_and_properties() {
    let account = AuthlibAccount::new(
        Uuid::new_v4(),
        "Steve",
        Uuid::parse_str("5627dd98e6be3c21b8a8e92344183641").unwrap(),
        "https://example.com/api/yggdrasil/",
        Some("Example".to_owned()),
    )
    .unwrap();
    let launch = LaunchAccount::from_authlib(
        &account,
        "third-party-token",
        r#"{"preferredLanguage":["zh_CN"]}"#,
    );

    assert_eq!(launch.username, "Steve");
    assert_eq!(launch.uuid, account.uuid());
    assert_eq!(launch.access_token, "third-party-token");
    assert_eq!(launch.user_type, "msa");
    assert_eq!(launch.user_properties, r#"{"preferredLanguage":["zh_CN"]}"#);
    assert!(!format!("{launch:?}").contains("third-party-token"));
}

#[test]
fn authlib_injector_arguments_are_added_before_the_main_class() {
    let temporary = tempfile::tempdir().unwrap();
    let jar = temporary.path().join("authlib-injector.jar");
    std::fs::write(&jar, b"jar").unwrap();
    let mut options = options();
    options.authlib = Some(AuthlibLaunchConfig {
        jar_path: jar.clone(),
        api_root: "https://example.com/api/yggdrasil/".to_owned(),
        prefetched_metadata: Some("eyJtZXRhIjp7fX0=".to_owned()),
    });

    let plan = LaunchPlan::from_options(&modern_version(), options).unwrap();
    let javaagent = format!(
        "-javaagent:{}={}",
        std::path::absolute(&jar).unwrap().display(),
        "https://example.com/api/yggdrasil/"
    );
    let injector_index = plan
        .jvm_arguments
        .iter()
        .position(|argument| argument == &javaagent)
        .expect("javaagent argument");
    assert!(
        plan.jvm_arguments
            .iter()
            .any(|argument| argument == "-Dauthlibinjector.side=client")
    );
    assert!(
        plan.jvm_arguments.iter().any(|argument| {
            argument == "-Dauthlibinjector.yggdrasil.prefetched=eyJtZXRhIjp7fX0="
        })
    );
    assert!(injector_index < plan.jvm_arguments.len());
    assert_eq!(
        plan.jvm_arguments[injector_index + 1],
        "-Dauthlibinjector.side=client"
    );
}

#[test]
fn missing_authlib_injector_jar_is_rejected() {
    let mut options = options();
    options.authlib = Some(AuthlibLaunchConfig {
        jar_path: PathBuf::from("does-not-exist/authlib-injector.jar"),
        api_root: "https://example.com/api/yggdrasil/".to_owned(),
        prefetched_metadata: None,
    });

    assert!(matches!(
        LaunchPlan::from_options(&modern_version(), options),
        Err(LaunchError::AuthlibInjectorMissing { .. })
    ));
}

#[test]
fn legacy_tokenizer_handles_quotes_and_empty_arguments() {
    let arguments =
        rmcl_launch::tokenize_legacy_arguments("--name 'Steve Smith' --empty \"\"").unwrap();
    assert_eq!(arguments, ["--name", "Steve Smith", "--empty", ""]);
}

fn server_options(version: &str, address: &str) -> LaunchOptions {
    let mut options = options();
    options.minecraft_version = version.to_owned();
    options.server = Some(ServerAddress::parse(address).unwrap());
    options
}

fn world_options(version: &str, world: &str) -> LaunchOptions {
    let mut options = options();
    options.minecraft_version = version.to_owned();
    options.quick_play_world = Some(world.to_owned());
    options
}

#[test]
fn modern_versions_enter_worlds_with_quick_play() {
    let plan =
        LaunchPlan::from_options(&modern_version(), world_options("1.21.1", "Survival")).unwrap();
    assert!(
        plan.game_arguments
            .windows(2)
            .any(|arguments| arguments == ["--quickPlaySingleplayer", "Survival"])
    );
}

#[test]
fn legacy_versions_reject_quick_play_worlds() {
    let error = LaunchPlan::from_options(&modern_version(), world_options("1.19.4", "Survival"))
        .unwrap_err();
    assert!(matches!(error, LaunchError::InvalidArguments(_)));
}

#[test]
fn modern_versions_join_servers_with_quick_play() {
    let plan = LaunchPlan::from_options(
        &modern_version(),
        server_options("1.21.1", "play.example.com"),
    )
    .unwrap();

    let index = plan
        .game_arguments
        .iter()
        .position(|argument| argument == "--quickPlayMultiplayer")
        .unwrap();
    assert_eq!(plan.game_arguments[index + 1], "play.example.com");
    assert!(
        !plan
            .game_arguments
            .iter()
            .any(|argument| argument == "--server")
    );
}

#[test]
fn legacy_versions_join_servers_with_server_and_port() {
    let plan = LaunchPlan::from_options(
        &modern_version(),
        server_options("1.19.4", "play.example.com:25566"),
    )
    .unwrap();

    let index = plan
        .game_arguments
        .iter()
        .position(|argument| argument == "--server")
        .unwrap();
    assert_eq!(
        &plan.game_arguments[index + 1..index + 4],
        ["play.example.com", "--port", "25566"]
    );
    assert!(
        !plan
            .game_arguments
            .iter()
            .any(|argument| argument == "--quickPlayMultiplayer")
    );
}

#[test]
fn quick_play_version_boundary_covers_snapshots_and_prereleases() {
    let version = modern_version();
    for applicable in ["1.20", "1.20-pre1", "23w14a", "24w14a", "26.3"] {
        let plan =
            LaunchPlan::from_options(&version, server_options(applicable, "play.example.com"))
                .unwrap();
        assert!(
            plan.game_arguments
                .iter()
                .any(|argument| argument == "--quickPlayMultiplayer"),
            "{applicable} should use Quick Play"
        );
    }
    for legacy in ["1.19.4", "23w13a", "1.7.10"] {
        let plan =
            LaunchPlan::from_options(&version, server_options(legacy, "play.example.com")).unwrap();
        assert!(
            plan.game_arguments
                .iter()
                .any(|argument| argument == "--server"),
            "{legacy} should use legacy server arguments"
        );
    }
}

#[test]
fn quick_play_preserves_explicit_ports_and_ipv6_brackets() {
    let plan = LaunchPlan::from_options(&modern_version(), server_options("1.21.1", "[::1]:25566"))
        .unwrap();
    assert!(
        plan.game_arguments
            .windows(2)
            .any(|arguments| arguments == ["--quickPlayMultiplayer", "[::1]:25566"])
    );

    let plan = LaunchPlan::from_options(
        &modern_version(),
        server_options("1.21.1", "play.example.com:25566"),
    )
    .unwrap();
    assert!(
        plan.game_arguments
            .windows(2)
            .any(|arguments| arguments == ["--quickPlayMultiplayer", "play.example.com:25566"])
    );
}
