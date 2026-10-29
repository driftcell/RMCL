//! Metadata read from resource and shader pack archives.
//!
//! Packs are user files and can be arbitrarily large or malformed. Metadata
//! reads therefore stay bounded, never extract the whole archive, and report
//! invalid entries as [`ContentError::PackMetadata`].

use std::{
    fmt,
    fs::{self, File},
    io::Read,
    path::Path,
};

use serde::Deserialize;
use serde_json::Value;
use zip::{ZipArchive, result::ZipError};

use crate::{ContentError, ContentKind};

const MAX_PACK_META_BYTES: u64 = 512 * 1024;
const MAX_PACK_ICON_BYTES: u64 = 4 * 1024 * 1024;
const MAX_PACK_DESCRIPTION_BYTES: u64 = 256 * 1024;
const PACK_META_FILE: &str = "pack.mcmeta";
const PACK_ICON_FILE: &str = "pack.png";
const SHADER_PACK_FILE: &str = "shaders/pack.json";
const CLIENT_VERSION_FILE: &str = "version.json";

/// A resource-pack format version.
///
/// Minecraft 1.21.9 introduced a minor version; older formats have minor `0`.
/// A `minor` of [`u32::MAX`] means "any minor version" when it is the upper
/// bound of a legacy range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PackFormat {
    pub major: u32,
    pub minor: u32,
}

impl PackFormat {
    pub const fn new(major: u32, minor: u32) -> Self {
        Self { major, minor }
    }

    /// A legacy major-only format.
    pub const fn major(major: u32) -> Self {
        Self { major, minor: 0 }
    }
}

impl fmt::Display for PackFormat {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.minor == 0 || self.minor == u32::MAX {
            write!(formatter, "{}", self.major)
        } else {
            write!(formatter, "{}.{}", self.major, self.minor)
        }
    }
}

/// One inclusive range of compatible resource-pack formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackFormatRange {
    pub min_inclusive: PackFormat,
    pub max_inclusive: PackFormat,
}

impl PackFormatRange {
    pub const fn new(min_inclusive: PackFormat, max_inclusive: PackFormat) -> Self {
        Self {
            min_inclusive,
            max_inclusive,
        }
    }

    pub const fn contains(self, format: PackFormat) -> bool {
        self.min_inclusive.major <= format.major
            && format.major <= self.max_inclusive.major
            && (format.major != self.min_inclusive.major
                || format.minor >= self.min_inclusive.minor)
            && (format.major != self.max_inclusive.major
                || format.minor <= self.max_inclusive.minor)
    }
}

/// Metadata declared by a resource pack's `pack.mcmeta`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourcePackMetadata {
    /// `pack_format` when present. Modern packs may use only `min_format` and
    /// `max_format`; the derived range is still exposed through
    /// [`Self::supported_formats`].
    pub pack_format: Option<PackFormat>,
    /// Explicit compatible formats. Empty means the legacy `pack_format`
    /// major version, when present.
    pub supported_formats: Vec<PackFormatRange>,
    /// Plain text extracted from a string or JSON text component.
    pub description: String,
    /// Raw `pack.png` bytes, bounded by [`MAX_PACK_ICON_BYTES`].
    pub icon_png: Option<Vec<u8>>,
}

impl ResourcePackMetadata {
    /// Returns whether the pack declares compatibility with `format`.
    pub fn supports(&self, format: PackFormat) -> bool {
        if !self.supported_formats.is_empty() {
            return self
                .supported_formats
                .iter()
                .any(|range| range.contains(format));
        }
        self.pack_format
            .is_some_and(|pack_format| pack_format.major == format.major)
    }

    /// Compares the pack with the target resource format of an instance.
    ///
    /// `None` means the target format is unknown (for example, the client JAR
    /// is missing), so compatibility cannot be decided without guessing.
    pub fn compatibility(&self, target_format: Option<PackFormat>) -> ResourcePackCompatibility {
        match target_format {
            Some(target) if self.supports(target) => ResourcePackCompatibility::Compatible,
            Some(_) => ResourcePackCompatibility::Incompatible,
            None => ResourcePackCompatibility::Unknown,
        }
    }
}

/// Result of comparing a resource pack with an instance's target format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourcePackCompatibility {
    Compatible,
    Incompatible,
    Unknown,
}

/// Metadata read from a shader pack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShaderPackMetadata {
    /// Whether the archive or directory contains a `shaders/` tree.
    pub has_shaders_directory: bool,
    /// Optional `shaderDescription` from `shaders/pack.json`.
    pub description: Option<String>,
}

/// Reads resource-pack metadata from a ZIP file or a pack directory.
pub fn read_resource_pack_metadata(
    path: impl AsRef<Path>,
) -> Result<ResourcePackMetadata, ContentError> {
    let path = path.as_ref();
    let metadata = fs::symlink_metadata(path).map_err(|source| ContentError::Io {
        operation: "inspect resource pack",
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.file_type().is_symlink() {
        return Err(ContentError::Symlink {
            path: path.to_path_buf(),
        });
    }
    if metadata.is_dir() {
        read_resource_pack_directory(path)
    } else if metadata.is_file() {
        read_resource_pack_zip(path)
    } else {
        Err(ContentError::UnsupportedFile {
            kind: ContentKind::ResourcePack,
            path: path.to_path_buf(),
        })
    }
}

/// Reads shader-pack metadata from a ZIP file or a pack directory.
pub fn read_shader_pack_metadata(
    path: impl AsRef<Path>,
) -> Result<ShaderPackMetadata, ContentError> {
    let path = path.as_ref();
    let metadata = fs::symlink_metadata(path).map_err(|source| ContentError::Io {
        operation: "inspect shader pack",
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.file_type().is_symlink() {
        return Err(ContentError::Symlink {
            path: path.to_path_buf(),
        });
    }
    if metadata.is_dir() {
        read_shader_pack_directory(path)
    } else if metadata.is_file() {
        read_shader_pack_zip(path)
    } else {
        Err(ContentError::UnsupportedFile {
            kind: ContentKind::ShaderPack,
            path: path.to_path_buf(),
        })
    }
}

/// Reads the resource-pack format declared by a Minecraft client JAR.
///
/// Modern clients store it in `version.json` as
/// `pack_version.resource_major`; older clients use a numeric
/// `pack_version`. A missing JAR or `version.json` yields `None` instead of an
/// error because compatibility can still be reported as unknown.
pub fn read_client_resource_pack_format(
    client_jar: impl AsRef<Path>,
) -> Result<Option<PackFormat>, ContentError> {
    let client_jar = client_jar.as_ref();
    let file = match File::open(client_jar) {
        Ok(file) => file,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(ContentError::Io {
                operation: "open Minecraft client JAR",
                path: client_jar.to_path_buf(),
                source,
            });
        }
    };
    let mut archive = ZipArchive::new(file).map_err(|source| ContentError::ClientMetadata {
        path: client_jar.to_path_buf(),
        reason: source.to_string(),
    })?;
    let Some(bytes) = read_zip_entry(&mut archive, CLIENT_VERSION_FILE, MAX_PACK_META_BYTES)
        .map_err(|reason| ContentError::ClientMetadata {
            path: client_jar.to_path_buf(),
            reason,
        })?
    else {
        return Ok(None);
    };
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|source| ContentError::ClientMetadata {
            path: client_jar.to_path_buf(),
            reason: format!("invalid version.json: {source}"),
        })?;
    Ok(pack_format_from_version_json(&value))
}

fn read_resource_pack_directory(path: &Path) -> Result<ResourcePackMetadata, ContentError> {
    let meta_path = path.join(PACK_META_FILE);
    let bytes = read_file_entry(&meta_path, MAX_PACK_META_BYTES, ContentKind::ResourcePack)?
        .ok_or_else(|| ContentError::PackMetadata {
            kind: ContentKind::ResourcePack,
            path: meta_path.clone(),
            reason: "missing pack.mcmeta".to_owned(),
        })?;
    let icon = read_optional_file(&path.join(PACK_ICON_FILE), MAX_PACK_ICON_BYTES);
    parse_resource_pack_meta(&meta_path, &bytes, icon)
}

fn read_resource_pack_zip(path: &Path) -> Result<ResourcePackMetadata, ContentError> {
    let mut archive = open_zip(path, ContentKind::ResourcePack)?;
    let bytes = read_zip_entry(&mut archive, PACK_META_FILE, MAX_PACK_META_BYTES)
        .map_err(|reason| ContentError::PackMetadata {
            kind: ContentKind::ResourcePack,
            path: path.to_path_buf(),
            reason,
        })?
        .ok_or_else(|| ContentError::PackMetadata {
            kind: ContentKind::ResourcePack,
            path: path.to_path_buf(),
            reason: "missing pack.mcmeta".to_owned(),
        })?;
    let icon = read_zip_entry(&mut archive, PACK_ICON_FILE, MAX_PACK_ICON_BYTES)
        .ok()
        .flatten();
    parse_resource_pack_meta(path, &bytes, icon)
}

fn read_shader_pack_directory(path: &Path) -> Result<ShaderPackMetadata, ContentError> {
    let shaders_path = path.join("shaders");
    let has_shaders_directory = fs::symlink_metadata(&shaders_path)
        .map(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
        .unwrap_or(false);
    let description = if has_shaders_directory {
        read_file_entry(
            &shaders_path.join("pack.json"),
            MAX_PACK_DESCRIPTION_BYTES,
            ContentKind::ShaderPack,
        )?
        .and_then(|bytes| parse_shader_description(&bytes))
    } else {
        None
    };
    Ok(ShaderPackMetadata {
        has_shaders_directory,
        description,
    })
}

fn read_shader_pack_zip(path: &Path) -> Result<ShaderPackMetadata, ContentError> {
    let mut archive = open_zip(path, ContentKind::ShaderPack)?;
    let has_shaders_directory = archive
        .file_names_raw()
        .any(|name| name == b"shaders/" || name.starts_with(b"shaders/"));
    let description = read_zip_entry(&mut archive, SHADER_PACK_FILE, MAX_PACK_DESCRIPTION_BYTES)
        .map_err(|reason| ContentError::PackMetadata {
            kind: ContentKind::ShaderPack,
            path: path.to_path_buf(),
            reason,
        })?
        .and_then(|bytes| parse_shader_description(&bytes));
    Ok(ShaderPackMetadata {
        has_shaders_directory,
        description,
    })
}

fn parse_resource_pack_meta(
    path: &Path,
    bytes: &[u8],
    icon_png: Option<Vec<u8>>,
) -> Result<ResourcePackMetadata, ContentError> {
    let file: PackMetaFile =
        serde_json::from_slice(bytes).map_err(|source| ContentError::PackMetadata {
            kind: ContentKind::ResourcePack,
            path: path.to_path_buf(),
            reason: format!("invalid pack.mcmeta: {source}"),
        })?;
    let supported_formats = parse_supported_formats(path, &file.pack)?;
    Ok(ResourcePackMetadata {
        pack_format: file.pack.pack_format.map(PackFormat::major),
        supported_formats,
        description: file
            .pack
            .description
            .as_ref()
            .map(plain_text)
            .unwrap_or_default(),
        icon_png,
    })
}

#[derive(Debug, Deserialize)]
struct PackMetaFile {
    pack: PackMetaSection,
}

#[derive(Debug, Deserialize)]
struct PackMetaSection {
    #[serde(default)]
    pack_format: Option<u32>,
    #[serde(default)]
    supported_formats: Option<Value>,
    #[serde(default)]
    min_format: Option<Value>,
    #[serde(default)]
    max_format: Option<Value>,
    #[serde(default)]
    description: Option<Value>,
}

fn parse_supported_formats(
    path: &Path,
    pack: &PackMetaSection,
) -> Result<Vec<PackFormatRange>, ContentError> {
    if let Some(value) = &pack.supported_formats {
        return parse_supported_formats_value(path, value);
    }
    match (&pack.min_format, &pack.max_format) {
        (Some(min), Some(max)) => {
            let min = parse_format_bound(path, min, false)?;
            let max = parse_format_bound(path, max, true)?;
            checked_range(path, min, max).map(|range| vec![range])
        }
        _ => Ok(Vec::new()),
    }
}

fn parse_supported_formats_value(
    path: &Path,
    value: &Value,
) -> Result<Vec<PackFormatRange>, ContentError> {
    match value {
        Value::Number(number) => number
            .as_u64()
            .and_then(|format| u32::try_from(format).ok())
            .map(|format| vec![major_range(format)])
            .ok_or_else(|| invalid_format(path, "supported_formats contains an invalid number")),
        Value::Object(_) => parse_format_object(path, value).map(|range| vec![range]),
        Value::Array(items) => {
            let mut ranges = Vec::new();
            for item in items {
                match item {
                    Value::Number(number) => {
                        let format = number
                            .as_u64()
                            .and_then(|format| u32::try_from(format).ok())
                            .ok_or_else(|| {
                                invalid_format(path, "supported_formats contains an invalid number")
                            })?;
                        ranges.push(major_range(format));
                    }
                    Value::Object(_) => ranges.push(parse_format_object(path, item)?),
                    Value::Array(_) => {
                        let format = parse_format_bound(path, item, false)?;
                        ranges.push(PackFormatRange::new(format, format));
                    }
                    _ => {
                        return Err(invalid_format(
                            path,
                            "supported_formats contains an unsupported entry",
                        ));
                    }
                }
            }
            Ok(ranges)
        }
        _ => Err(invalid_format(
            path,
            "supported_formats must be a number, range, or array",
        )),
    }
}

fn parse_format_object(path: &Path, value: &Value) -> Result<PackFormatRange, ContentError> {
    let min = value
        .get("min_inclusive")
        .or_else(|| value.get("min_format"))
        .ok_or_else(|| invalid_format(path, "format range has no min_inclusive"))?;
    let max = value
        .get("max_inclusive")
        .or_else(|| value.get("max_format"))
        .ok_or_else(|| invalid_format(path, "format range has no max_inclusive"))?;
    let min = parse_format_bound(path, min, false)?;
    let max = parse_format_bound(path, max, true)?;
    checked_range(path, min, max)
}

/// Parses a pack format bound: an integer or a `[major, minor]` pair.
///
/// A single integer upper bound means "any minor", matching the 1.21.9+
/// `max_format` semantics.
fn parse_format_bound(
    path: &Path,
    value: &Value,
    upper_bound: bool,
) -> Result<PackFormat, ContentError> {
    match value {
        Value::Number(number) => {
            let major = number
                .as_u64()
                .and_then(|format| u32::try_from(format).ok())
                .ok_or_else(|| invalid_format(path, "format version is not a valid integer"))?;
            let minor = if upper_bound { u32::MAX } else { 0 };
            Ok(PackFormat::new(major, minor))
        }
        Value::Array(items) if (1..=2).contains(&items.len()) => {
            let major = items
                .first()
                .and_then(Value::as_u64)
                .and_then(|format| u32::try_from(format).ok())
                .ok_or_else(|| invalid_format(path, "format version has no valid major part"))?;
            let minor = match items.get(1) {
                Some(value) => value
                    .as_u64()
                    .and_then(|format| u32::try_from(format).ok())
                    .ok_or_else(|| {
                        invalid_format(path, "format version has no valid minor part")
                    })?,
                None if upper_bound => u32::MAX,
                None => 0,
            };
            Ok(PackFormat::new(major, minor))
        }
        _ => Err(invalid_format(
            path,
            "format version must be an integer or [major, minor]",
        )),
    }
}

fn major_range(format: u32) -> PackFormatRange {
    PackFormatRange::new(PackFormat::major(format), PackFormat::new(format, u32::MAX))
}

fn checked_range(
    path: &Path,
    min: PackFormat,
    max: PackFormat,
) -> Result<PackFormatRange, ContentError> {
    if min > max {
        return Err(invalid_format(
            path,
            "format range has min_inclusive greater than max_inclusive",
        ));
    }
    Ok(PackFormatRange::new(min, max))
}

fn plain_text(value: &Value) -> String {
    let mut text = String::new();
    append_text(value, &mut text);
    sanitize_minecraft_text(text.trim())
}

fn append_text(value: &Value, text: &mut String) {
    match value {
        Value::String(value) => text.push_str(value),
        Value::Array(values) => {
            for value in values {
                append_text(value, text);
            }
        }
        Value::Object(value) => {
            if let Some(value) = value.get("text").and_then(Value::as_str) {
                text.push_str(value);
            } else if let Some(value) = value.get("translate").and_then(Value::as_str) {
                text.push_str(value);
            }
            if let Some(extra) = value.get("extra") {
                append_text(extra, text);
            }
        }
        _ => {}
    }
}

fn sanitize_minecraft_text(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(character) = chars.next() {
        if character == '\u{00a7}' {
            chars.next();
        } else {
            result.push(character);
        }
    }
    result.trim().to_owned()
}

fn parse_shader_description(bytes: &[u8]) -> Option<String> {
    let value: Value = serde_json::from_slice(bytes).ok()?;
    let description = value.get("shaderDescription")?.as_str()?;
    Some(sanitize_minecraft_text(description))
}

fn pack_format_from_version_json(value: &Value) -> Option<PackFormat> {
    let pack_version = value.get("pack_version")?;
    if let Some(format) = pack_version.as_u64() {
        return u32::try_from(format).ok().map(PackFormat::major);
    }
    let major = pack_version
        .get("resource_major")
        .and_then(Value::as_u64)
        .and_then(|format| u32::try_from(format).ok())?;
    let minor = pack_version
        .get("resource_minor")
        .and_then(Value::as_u64)
        .and_then(|format| u32::try_from(format).ok())
        .unwrap_or(0);
    Some(PackFormat::new(major, minor))
}

fn open_zip(path: &Path, kind: ContentKind) -> Result<ZipArchive<File>, ContentError> {
    let file = File::open(path).map_err(|source| ContentError::Io {
        operation: "open content archive",
        path: path.to_path_buf(),
        source,
    })?;
    ZipArchive::new(file).map_err(|source| ContentError::PackMetadata {
        kind,
        path: path.to_path_buf(),
        reason: source.to_string(),
    })
}

fn read_file_entry(
    path: &Path,
    limit: u64,
    kind: ContentKind,
) -> Result<Option<Vec<u8>>, ContentError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(ContentError::Io {
                operation: "inspect content metadata file",
                path: path.to_path_buf(),
                source,
            });
        }
    };
    if metadata.file_type().is_symlink() {
        return Err(ContentError::Symlink {
            path: path.to_path_buf(),
        });
    }
    if metadata.len() > limit {
        return Err(ContentError::PackMetadata {
            kind,
            path: path.to_path_buf(),
            reason: format!("metadata exceeds {limit} bytes"),
        });
    }
    let bytes = fs::read(path).map_err(|source| ContentError::Io {
        operation: "read content metadata file",
        path: path.to_path_buf(),
        source,
    })?;
    if bytes.len() as u64 > limit {
        return Err(ContentError::PackMetadata {
            kind,
            path: path.to_path_buf(),
            reason: format!("metadata exceeds {limit} bytes"),
        });
    }
    Ok(Some(bytes))
}

fn read_optional_file(path: &Path, limit: u64) -> Option<Vec<u8>> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > limit {
        return None;
    }
    fs::read(path)
        .ok()
        .filter(|bytes| bytes.len() as u64 <= limit)
}

fn read_zip_entry(
    archive: &mut ZipArchive<File>,
    entry: &str,
    limit: u64,
) -> Result<Option<Vec<u8>>, String> {
    let file = match archive.by_name(entry) {
        Ok(file) => file,
        Err(ZipError::FileNotFound) => return Ok(None),
        Err(source) => return Err(format!("cannot read `{entry}`: {source}")),
    };
    if file.size() > limit {
        return Err(format!("`{entry}` exceeds {limit} bytes"));
    }
    let mut bytes = Vec::with_capacity(file.size() as usize);
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| format!("cannot read `{entry}`: {source}"))?;
    if bytes.len() as u64 > limit {
        return Err(format!("`{entry}` exceeds {limit} bytes"));
    }
    Ok(Some(bytes))
}

fn invalid_format(path: &Path, reason: &str) -> ContentError {
    ContentError::PackMetadata {
        kind: ContentKind::ResourcePack,
        path: path.to_path_buf(),
        reason: reason.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, io::Write, path::Path};

    use tempfile::TempDir;
    use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

    use super::*;

    fn zip(path: &Path, entries: &[(&str, &[u8])]) {
        let file = File::create(path).unwrap();
        let mut archive = ZipWriter::new(file);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        for (name, contents) in entries {
            archive.start_file(*name, options).unwrap();
            archive.write_all(contents).unwrap();
        }
        archive.finish().unwrap();
    }

    fn resource_pack_meta() -> &'static str {
        r#"{
            "pack": {
                "pack_format": 15,
                "supported_formats": {"min_inclusive": 15, "max_inclusive": 75},
                "description": {"text": "§aFresh", "extra": [{"text": " Animations"}]}
            }
        }"#
    }

    #[test]
    fn reads_resource_pack_metadata_from_zip_and_directory() {
        let temporary = TempDir::new().unwrap();
        let archive_path = temporary.path().join("pack.zip");
        zip(
            &archive_path,
            &[
                ("pack.mcmeta", resource_pack_meta().as_bytes()),
                ("pack.png", b"png"),
            ],
        );
        let archive = read_resource_pack_metadata(&archive_path).unwrap();
        assert_eq!(archive.pack_format, Some(PackFormat::major(15)));
        assert_eq!(
            archive.supported_formats,
            vec![PackFormatRange::new(
                PackFormat::major(15),
                PackFormat::new(75, u32::MAX),
            )]
        );
        assert_eq!(archive.description, "Fresh Animations");
        assert_eq!(archive.icon_png.as_deref(), Some(&b"png"[..]));
        assert_eq!(
            archive.compatibility(Some(PackFormat::major(75))),
            ResourcePackCompatibility::Compatible
        );
        assert_eq!(
            archive.compatibility(Some(PackFormat::major(14))),
            ResourcePackCompatibility::Incompatible
        );
        assert_eq!(
            archive.compatibility(None),
            ResourcePackCompatibility::Unknown
        );

        let directory = temporary.path().join("folder");
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("pack.mcmeta"), resource_pack_meta()).unwrap();
        fs::write(directory.join("pack.png"), b"folder png").unwrap();
        let folder = read_resource_pack_metadata(&directory).unwrap();
        assert_eq!(folder.description, "Fresh Animations");
        assert_eq!(folder.icon_png.as_deref(), Some(&b"folder png"[..]));
    }

    #[test]
    fn reads_supported_format_arrays_and_legacy_min_max() {
        let temporary = TempDir::new().unwrap();
        let array_path = temporary.path().join("array.zip");
        zip(
            &array_path,
            &[(
                "pack.mcmeta",
                br#"{"pack":{"pack_format":15,"supported_formats":[15,{"min_inclusive":30,"max_inclusive":40}]}}"#,
            )],
        );
        let metadata = read_resource_pack_metadata(&array_path).unwrap();
        assert_eq!(
            metadata.supported_formats,
            vec![
                PackFormatRange::new(PackFormat::major(15), PackFormat::new(15, u32::MAX),),
                PackFormatRange::new(PackFormat::major(30), PackFormat::new(40, u32::MAX),),
            ]
        );

        let legacy_path = temporary.path().join("legacy.zip");
        zip(
            &legacy_path,
            &[(
                "pack.mcmeta",
                br#"{"pack":{"pack_format":15,"min_format":15,"max_format":20}}"#,
            )],
        );
        let metadata = read_resource_pack_metadata(&legacy_path).unwrap();
        assert_eq!(
            metadata.supported_formats,
            vec![PackFormatRange::new(
                PackFormat::major(15),
                PackFormat::new(20, u32::MAX),
            )]
        );

        let modern_path = temporary.path().join("modern.zip");
        zip(
            &modern_path,
            &[(
                "pack.mcmeta",
                br#"{"pack":{"description":"Modern","min_format":[97,1],"max_format":[97,3]}}"#,
            )],
        );
        let metadata = read_resource_pack_metadata(&modern_path).unwrap();
        assert_eq!(metadata.pack_format, None);
        assert_eq!(
            metadata.supported_formats,
            vec![PackFormatRange::new(
                PackFormat::new(97, 1),
                PackFormat::new(97, 3),
            )]
        );
        assert_eq!(
            metadata.compatibility(Some(PackFormat::new(97, 2))),
            ResourcePackCompatibility::Compatible
        );
        assert_eq!(
            metadata.compatibility(Some(PackFormat::new(97, 4))),
            ResourcePackCompatibility::Incompatible
        );
    }

    #[test]
    fn rejects_missing_or_oversized_pack_metadata() {
        let temporary = TempDir::new().unwrap();
        let missing = temporary.path().join("missing.zip");
        zip(&missing, &[("pack.png", b"png")]);
        assert!(matches!(
            read_resource_pack_metadata(&missing),
            Err(ContentError::PackMetadata { .. })
        ));

        let directory = temporary.path().join("oversized");
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("pack.mcmeta"),
            vec![b'x'; (MAX_PACK_META_BYTES + 1) as usize],
        )
        .unwrap();
        assert!(matches!(
            read_resource_pack_metadata(&directory),
            Err(ContentError::PackMetadata { .. })
        ));
    }

    #[test]
    fn reads_shader_pack_metadata_from_zip_and_directory() {
        let temporary = TempDir::new().unwrap();
        let archive_path = temporary.path().join("shader.zip");
        zip(
            &archive_path,
            &[
                ("shaders/shaders.properties", b"profile.HIGH=1"),
                (
                    "shaders/pack.json",
                    r#"{"shaderDescription":"§bNice"}"#.as_bytes(),
                ),
            ],
        );
        let archive = read_shader_pack_metadata(&archive_path).unwrap();
        assert!(archive.has_shaders_directory);
        assert_eq!(archive.description.as_deref(), Some("Nice"));

        let directory = temporary.path().join("shader");
        fs::create_dir_all(directory.join("shaders")).unwrap();
        fs::write(
            directory.join("shaders/pack.json"),
            br#"{"shaderDescription":"Folder"}"#,
        )
        .unwrap();
        let folder = read_shader_pack_metadata(&directory).unwrap();
        assert!(folder.has_shaders_directory);
        assert_eq!(folder.description.as_deref(), Some("Folder"));

        let empty = temporary.path().join("empty");
        fs::create_dir_all(&empty).unwrap();
        let empty = read_shader_pack_metadata(&empty).unwrap();
        assert!(!empty.has_shaders_directory);
        assert!(empty.description.is_none());
    }

    #[test]
    fn reads_client_resource_pack_format_and_tolerates_missing_metadata() {
        let temporary = TempDir::new().unwrap();
        let modern = temporary.path().join("modern.jar");
        zip(
            &modern,
            &[(
                "version.json",
                br#"{"id":"26.3","pack_version":{"resource_major":97,"resource_minor":1}}"#,
            )],
        );
        assert_eq!(
            read_client_resource_pack_format(&modern).unwrap(),
            Some(PackFormat::new(97, 1))
        );

        let legacy = temporary.path().join("legacy.jar");
        zip(&legacy, &[("version.json", br#"{"pack_version":5}"#)]);
        assert_eq!(
            read_client_resource_pack_format(&legacy).unwrap(),
            Some(PackFormat::major(5))
        );

        let missing = temporary.path().join("missing.jar");
        zip(&missing, &[("other.txt", b"x")]);
        assert_eq!(read_client_resource_pack_format(&missing).unwrap(), None);
        assert_eq!(
            read_client_resource_pack_format(temporary.path().join("absent.jar")).unwrap(),
            None
        );
    }
}
