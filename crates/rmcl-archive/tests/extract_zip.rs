use std::{fs, io::Write, path::PathBuf};

use rmcl_archive::{ArchiveExtractor, ExtractionError, ExtractionLimits, ExtractionReport};
use rmcl_domain::ExtractionPlan;
use tempfile::{NamedTempFile, TempDir};
use tokio_util::sync::CancellationToken;
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

struct TestEntry {
    name: String,
    data: Vec<u8>,
    kind: EntryKind,
}

#[derive(Clone, Copy)]
enum EntryKind {
    File { mode: u32 },
    Directory,
    Symlink,
    Special { mode: u32 },
}

fn file(name: impl Into<String>, data: impl AsRef<[u8]>) -> TestEntry {
    file_with_mode(name, data, 0o100644)
}

fn file_with_mode(name: impl Into<String>, data: impl AsRef<[u8]>, mode: u32) -> TestEntry {
    TestEntry {
        name: name.into(),
        data: data.as_ref().to_vec(),
        kind: EntryKind::File { mode },
    }
}

fn directory(name: impl Into<String>) -> TestEntry {
    TestEntry {
        name: name.into(),
        data: Vec::new(),
        kind: EntryKind::Directory,
    }
}

fn symlink(name: impl Into<String>, target: impl AsRef<[u8]>) -> TestEntry {
    TestEntry {
        name: name.into(),
        data: target.as_ref().to_vec(),
        kind: EntryKind::Symlink,
    }
}

fn special(name: impl Into<String>, mode: u32) -> TestEntry {
    TestEntry {
        name: name.into(),
        data: b"special".to_vec(),
        kind: EntryKind::Special { mode },
    }
}

fn archive(entries: &[TestEntry]) -> NamedTempFile {
    let archive = NamedTempFile::new().unwrap();
    let mut writer = ZipWriter::new(archive.reopen().unwrap());
    for entry in entries {
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        match entry.kind {
            EntryKind::Directory => {
                writer
                    .add_directory(&entry.name, options.unix_permissions(0o40755))
                    .unwrap();
            }
            EntryKind::File { mode } => {
                writer
                    .start_file(&entry.name, options.unix_permissions(mode))
                    .unwrap();
                writer.write_all(&entry.data).unwrap();
            }
            EntryKind::Symlink => {
                writer
                    .start_file(
                        &entry.name,
                        options.external_attributes((0o120777_u32) << 16),
                    )
                    .unwrap();
                writer.write_all(&entry.data).unwrap();
            }
            EntryKind::Special { mode } => {
                writer
                    .start_file(
                        &entry.name,
                        options
                            .system(zip::System::Dos)
                            .external_attributes(mode << 16),
                    )
                    .unwrap();
                writer.write_all(&entry.data).unwrap();
            }
        }
    }
    writer.finish().unwrap();
    archive
}

fn request(source: &NamedTempFile, destination: PathBuf) -> ExtractionPlan {
    ExtractionPlan {
        source: source.path().to_path_buf(),
        destination,
        exclude_prefixes: Vec::new(),
    }
}

fn extract(
    source: &NamedTempFile,
    destination: PathBuf,
) -> Result<ExtractionReport, ExtractionError> {
    ArchiveExtractor::default().extract_zip(
        &request(source, destination),
        CancellationToken::new(),
        None,
    )
}

#[test]
fn extracts_safe_entries_with_exclusions_and_progress() {
    let archive = archive(&[
        directory("native/"),
        file("native/lib.so", b"abc"),
        file("META-INF/MANIFEST.MF", b"manifest"),
        file("docs/readme.txt", b"docs"),
    ]);
    let temporary = TempDir::new().unwrap();
    let destination = temporary.path().join("natives");
    let mut request = request(&archive, destination.clone());
    request.exclude_prefixes.push("META-INF/".to_owned());
    let mut events = Vec::new();
    let mut progress = |event| events.push(event);

    let report = ArchiveExtractor::default()
        .extract_zip(&request, CancellationToken::new(), Some(&mut progress))
        .unwrap();

    assert_eq!(report.files, 2);
    assert_eq!(report.directories, 1);
    assert_eq!(report.skipped, 1);
    assert_eq!(report.extracted_bytes, 7);
    assert_eq!(fs::read(destination.join("native/lib.so")).unwrap(), b"abc");
    assert_eq!(
        fs::read(destination.join("docs/readme.txt")).unwrap(),
        b"docs"
    );
    assert!(!destination.join("META-INF").exists());
    assert_eq!(events.len(), 4);
    assert_eq!(events.last().unwrap().completed_entries, 4);
    assert_eq!(events.last().unwrap().extracted_bytes, 7);
}

#[test]
fn rejects_unsafe_entry_names_without_publishing() {
    for name in [
        "../escape",
        "/absolute",
        "safe/../escape",
        "safe//file",
        "safe/./file",
        "C:/escape",
        "safe\\escape",
        "nul\0byte",
    ] {
        let archive = archive(&[file(name, b"bad")]);
        let temporary = TempDir::new().unwrap();
        let destination = temporary.path().join("output");

        let result = extract(&archive, destination.clone());

        assert!(matches!(result, Err(ExtractionError::UnsafeEntry { .. })));
        assert!(!destination.exists(), "published unsafe entry {name:?}");
        assert_eq!(fs::read_dir(temporary.path()).unwrap().count(), 0);
    }
}

#[test]
fn rejects_case_insensitive_duplicates() {
    let archive = archive(&[file("Native/A.DLL", b"one"), file("native/a.dll", b"two")]);
    let temporary = TempDir::new().unwrap();
    let destination = temporary.path().join("output");

    let result = extract(&archive, destination.clone());

    assert!(matches!(
        result,
        Err(ExtractionError::DuplicateEntry { .. })
    ));
    assert!(!destination.exists());
}

#[test]
fn rejects_symlink_entries() {
    let archive = archive(&[symlink("native/link", "../escape")]);
    let temporary = TempDir::new().unwrap();
    let destination = temporary.path().join("output");

    let result = extract(&archive, destination.clone());

    assert!(matches!(
        result,
        Err(ExtractionError::UnsupportedEntry { .. })
    ));
    assert!(!destination.exists());
}

#[test]
fn rejects_non_regular_special_files() {
    let archive = archive(&[special("native/fifo", 0o010666)]);
    let temporary = TempDir::new().unwrap();
    let destination = temporary.path().join("output");

    let result = extract(&archive, destination.clone());

    assert!(matches!(
        result,
        Err(ExtractionError::UnsupportedEntry { .. })
    ));
    assert!(!destination.exists());
}

#[test]
fn merges_archives_and_preserves_existing_files() {
    let first = archive(&[file("native/a.so", b"one"), file("shared.txt", b"first")]);
    let second = archive(&[file("native/b.so", b"two"), file("shared.txt", b"second")]);
    let temporary = TempDir::new().unwrap();
    let destination = temporary.path().join("natives");
    let extractor = ArchiveExtractor::default();

    extractor
        .extract_zip(
            &request(&first, destination.clone()),
            CancellationToken::new(),
            None,
        )
        .unwrap();
    let report = extractor
        .merge_zip(
            &request(&second, destination.clone()),
            CancellationToken::new(),
            None,
        )
        .unwrap();

    assert_eq!(fs::read(destination.join("native/a.so")).unwrap(), b"one");
    assert_eq!(fs::read(destination.join("native/b.so")).unwrap(), b"two");
    assert_eq!(fs::read(destination.join("shared.txt")).unwrap(), b"first");
    assert_eq!(report.skipped, 1);
    assert_eq!(report.files, 2);
}

#[test]
fn merge_rejects_file_directory_conflicts_without_replacing() {
    let first = archive(&[file("a", b"one")]);
    let second = archive(&[file("new", b"new"), file("a/b", b"two")]);
    let temporary = TempDir::new().unwrap();
    let destination = temporary.path().join("output");
    let extractor = ArchiveExtractor::default();

    extractor
        .extract_zip(
            &request(&first, destination.clone()),
            CancellationToken::new(),
            None,
        )
        .unwrap();
    let result = extractor.merge_zip(
        &request(&second, destination.clone()),
        CancellationToken::new(),
        None,
    );

    assert!(matches!(result, Err(ExtractionError::DestinationExists(_))));
    assert_eq!(fs::read(destination.join("a")).unwrap(), b"one");
    assert!(!destination.join("new").exists());
}

#[test]
fn merge_publishes_a_missing_destination() {
    let archive = archive(&[file("native/a.so", b"one")]);
    let temporary = TempDir::new().unwrap();
    let destination = temporary.path().join("natives");

    ArchiveExtractor::default()
        .merge_zip(
            &request(&archive, destination.clone()),
            CancellationToken::new(),
            None,
        )
        .unwrap();

    assert_eq!(fs::read(destination.join("native/a.so")).unwrap(), b"one");
}

#[test]
fn merge_rejects_a_non_directory_destination() {
    let archive = archive(&[file("a", b"new")]);
    let temporary = TempDir::new().unwrap();
    let destination = temporary.path().join("output");
    fs::write(&destination, b"existing").unwrap();

    let result = ArchiveExtractor::default().merge_zip(
        &request(&archive, destination.clone()),
        CancellationToken::new(),
        None,
    );

    assert!(matches!(result, Err(ExtractionError::DestinationExists(_))));
    assert_eq!(fs::read(destination).unwrap(), b"existing");
}

#[test]
fn enforces_entry_count_limit() {
    let archive = archive(&[file("a", b"a"), file("b", b"b")]);
    let temporary = TempDir::new().unwrap();
    let destination = temporary.path().join("output");
    let extractor = ArchiveExtractor::new(ExtractionLimits {
        max_entries: 1,
        ..ExtractionLimits::default()
    });

    let result = extractor.extract_zip(
        &request(&archive, destination.clone()),
        CancellationToken::new(),
        None,
    );

    assert!(matches!(
        result,
        Err(ExtractionError::TooManyEntries { .. })
    ));
    assert!(!destination.exists());
}

#[test]
fn enforces_single_entry_size_limit() {
    let archive = archive(&[file("large.bin", b"1234")]);
    let temporary = TempDir::new().unwrap();
    let destination = temporary.path().join("output");
    let extractor = ArchiveExtractor::new(ExtractionLimits {
        max_entry_size: 3,
        ..ExtractionLimits::default()
    });

    let result = extractor.extract_zip(
        &request(&archive, destination.clone()),
        CancellationToken::new(),
        None,
    );

    assert!(matches!(result, Err(ExtractionError::EntryTooLarge { .. })));
    assert!(!destination.exists());
}

#[test]
fn enforces_total_size_limit_and_cleans_staging() {
    let archive = archive(&[file("a.bin", b"1234"), file("b.bin", b"5678")]);
    let temporary = TempDir::new().unwrap();
    let destination = temporary.path().join("output");
    let extractor = ArchiveExtractor::new(ExtractionLimits {
        max_total_size: 5,
        ..ExtractionLimits::default()
    });

    let result = extractor.extract_zip(
        &request(&archive, destination.clone()),
        CancellationToken::new(),
        None,
    );

    assert!(matches!(result, Err(ExtractionError::TotalTooLarge { .. })));
    assert!(!destination.exists());
    assert_eq!(fs::read_dir(temporary.path()).unwrap().count(), 0);
}

#[test]
fn cancellation_before_extraction_does_not_publish() {
    let archive = archive(&[file("a", b"a")]);
    let temporary = TempDir::new().unwrap();
    let destination = temporary.path().join("output");
    let cancellation = CancellationToken::new();
    cancellation.cancel();

    let result = ArchiveExtractor::default().extract_zip(
        &request(&archive, destination.clone()),
        cancellation,
        None,
    );

    assert!(matches!(result, Err(ExtractionError::Cancelled)));
    assert!(!destination.exists());
}

#[test]
fn cancellation_between_entries_cleans_staging() {
    let archive = archive(&[file("a", b"a"), file("b", b"b")]);
    let temporary = TempDir::new().unwrap();
    let destination = temporary.path().join("output");
    let cancellation = CancellationToken::new();
    let cancel = cancellation.clone();
    let mut progress = move |_| cancel.cancel();

    let result = ArchiveExtractor::default().extract_zip(
        &request(&archive, destination.clone()),
        cancellation,
        Some(&mut progress),
    );

    assert!(matches!(result, Err(ExtractionError::Cancelled)));
    assert!(!destination.exists());
    assert_eq!(fs::read_dir(temporary.path()).unwrap().count(), 0);
}

#[test]
fn rejects_existing_destination() {
    let archive = archive(&[file("a", b"a")]);
    let temporary = TempDir::new().unwrap();
    let destination = temporary.path().join("output");
    fs::create_dir(&destination).unwrap();
    fs::write(destination.join("keep.txt"), b"keep").unwrap();

    let result = extract(&archive, destination.clone());

    assert!(matches!(result, Err(ExtractionError::DestinationExists(_))));
    assert_eq!(fs::read(destination.join("keep.txt")).unwrap(), b"keep");
}

#[test]
fn detects_corrupted_content_and_does_not_publish() {
    let archive = archive(&[file("data.bin", b"hello")]);
    let mut bytes = fs::read(archive.path()).unwrap();
    let position = bytes
        .windows(5)
        .position(|window| window == b"hello")
        .unwrap();
    bytes[position] = b'x';
    fs::write(archive.path(), bytes).unwrap();
    let temporary = TempDir::new().unwrap();
    let destination = temporary.path().join("output");

    let result = extract(&archive, destination.clone());

    assert!(result.is_err());
    assert!(!destination.exists());
    assert_eq!(fs::read_dir(temporary.path()).unwrap().count(), 0);
}

#[test]
fn publishes_an_empty_archive_as_an_empty_directory() {
    let archive = archive(&[]);
    let temporary = TempDir::new().unwrap();
    let destination = temporary.path().join("output");

    let report = extract(&archive, destination.clone()).unwrap();

    assert_eq!(report.files, 0);
    assert_eq!(report.extracted_bytes, 0);
    assert!(destination.is_dir());
    assert_eq!(fs::read_dir(destination).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn preserves_executable_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let archive = archive(&[file_with_mode("native/run.so", b"binary", 0o100755)]);
    let temporary = TempDir::new().unwrap();
    let destination = temporary.path().join("output");

    extract(&archive, destination.clone()).unwrap();

    let mode = fs::metadata(destination.join("native/run.so"))
        .unwrap()
        .permissions()
        .mode();
    assert_ne!(mode & 0o111, 0);
}
