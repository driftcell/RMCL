use std::path::Path;

use rmcl_app::{
    ConflictPolicy, ContentKind, IsolationMode, delete_instance_content, import_instance_content,
    scan_instance_content, set_instance_content_enabled,
};
use rmcl_instance::{InstanceLayout, MinecraftLayout, VersionId};
use tempfile::TempDir;

fn instance(root: &Path) -> InstanceLayout {
    let instance = MinecraftLayout::new(root)
        .instance(VersionId::new("1.21.1").unwrap(), IsolationMode::Isolated);
    instance.ensure_content_dirs().unwrap();
    instance
}

fn write(path: &Path, contents: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

#[tokio::test]
async fn imports_lists_toggles_and_deletes_instance_content() {
    let temporary = TempDir::new().unwrap();
    let instance = instance(&temporary.path().join(".minecraft"));
    let source = temporary.path().join("downloads/example.jar");
    write(&source, b"mod");

    let imported = import_instance_content(
        instance.clone(),
        ContentKind::Mod,
        source,
        ConflictPolicy::Rename,
    )
    .await
    .unwrap();

    assert_eq!(imported.relative_path.as_str(), "example.jar");
    assert!(imported.enabled);
    let library = scan_instance_content(instance.clone()).await.unwrap();
    assert_eq!(library.count(ContentKind::Mod), 1);

    let disabled = set_instance_content_enabled(
        instance.clone(),
        ContentKind::Mod,
        imported.relative_path,
        false,
    )
    .await
    .unwrap();
    assert!(!disabled.enabled);
    assert_eq!(disabled.relative_path.as_str(), "example.jar.disabled");

    let library = scan_instance_content(instance.clone()).await.unwrap();
    assert_eq!(library.count(ContentKind::Mod), 1);
    assert!(!library.items[0].enabled);

    delete_instance_content(instance.clone(), ContentKind::Mod, disabled.relative_path)
        .await
        .unwrap();
    assert!(
        scan_instance_content(instance)
            .await
            .unwrap()
            .items
            .is_empty()
    );
}
