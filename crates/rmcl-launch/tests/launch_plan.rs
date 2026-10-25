use std::path::PathBuf;

use rmcl_auth::OfflineAccount;
use rmcl_domain::{Architecture, OperatingSystem, Platform};
use rmcl_install::{InstallPlan, PlannedFile, PlannedFileKind};
use rmcl_instance::{IsolationMode, MinecraftLayout, VersionId};
use rmcl_java::{JavaRuntime, JavaRuntimeSource};
use rmcl_launch::{LaunchAccount, LaunchError, LaunchOptions, LaunchPlan};
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
fn legacy_tokenizer_handles_quotes_and_empty_arguments() {
    let arguments =
        rmcl_launch::tokenize_legacy_arguments("--name 'Steve Smith' --empty \"\"").unwrap();
    assert_eq!(arguments, ["--name", "Steve Smith", "--empty", ""]);
}
