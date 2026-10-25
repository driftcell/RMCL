use rmcl_instance::{MinecraftLayout, RepositoryPath, VersionId};
use rmcl_minecraft::{
    AssetCatalog, DownloadInfo, FeatureMap, GameVersion, Library, Platform, Sha1Hash,
    evaluate_rules,
};
use url::Url;

use crate::{
    ExtractionPlan, InstallPlan, InstallPlanError, MavenCoordinate, PlannedFile, PlannedFileKind,
    plan::InstallPlanBuilder,
};

const DEFAULT_LIBRARY_BASE: &str = "https://libraries.minecraft.net/";
const ASSET_OBJECT_BASE: &str = "https://resources.download.minecraft.net/";

/// Plans client, library, native, and asset-index files without asset objects.
pub fn plan_vanilla(
    layout: &MinecraftLayout,
    version: &GameVersion,
    platform: &Platform,
) -> Result<InstallPlan, InstallPlanError> {
    let id = VersionId::new(version.id.clone())?;
    if let Some(inherits_from) = &version.inherits_from {
        return Err(InstallPlanError::UnsupportedInheritance {
            id: version.id.clone(),
            inherits_from: inherits_from.clone(),
        });
    }
    if let Some(jar) = &version.jar {
        let jar_id = VersionId::new(jar.clone())?;
        if jar_id != id {
            return Err(InstallPlanError::UnsupportedClientJarRedirect {
                id: version.id.clone(),
                jar: jar.clone(),
            });
        }
    }

    let client = version
        .downloads
        .get("client")
        .ok_or_else(|| InstallPlanError::MissingClientDownload(version.id.clone()))?;
    let asset_index = version
        .asset_index
        .as_ref()
        .ok_or_else(|| InstallPlanError::MissingAssetIndex(version.id.clone()))?;
    let mut builder = InstallPlanBuilder::default();

    let client_url = required_url(client.url.as_deref(), &format!("client for {}", version.id))?;
    builder.push_file(PlannedFile {
        kind: PlannedFileKind::Client,
        destination: layout.client_jar_path(&id),
        sources: vec![client_url],
        sha1: optional_sha1(client.sha1.as_deref())?,
        size: client.size,
    })?;

    for library in &version.libraries {
        if !evaluate_rules(&library.rules, platform, &FeatureMap::new())? {
            continue;
        }
        plan_library(layout, &id, library, platform, &mut builder)?;
    }

    let index_url = parse_url(&asset_index.url, &format!("asset index {}", asset_index.id))?;
    builder.push_file(PlannedFile {
        kind: PlannedFileKind::AssetIndex,
        destination: layout.asset_index_path(&asset_index.id)?,
        sources: vec![index_url],
        sha1: optional_sha1(asset_index.sha1.as_deref())?,
        size: asset_index.size,
    })?;

    Ok(builder.finish())
}

/// Plans content-addressed asset objects after an asset index has been downloaded and parsed.
pub fn plan_asset_objects(
    layout: &MinecraftLayout,
    catalog: &AssetCatalog,
) -> Result<InstallPlan, InstallPlanError> {
    let base = parse_url(ASSET_OBJECT_BASE, "asset object base")?;
    plan_asset_objects_with_base(layout, catalog, &base)
}

/// Plans asset objects against an explicit source root without changing their integrity checks.
pub fn plan_asset_objects_with_base(
    layout: &MinecraftLayout,
    catalog: &AssetCatalog,
    base: &Url,
) -> Result<InstallPlan, InstallPlanError> {
    if catalog.is_virtual == Some(true) {
        return Err(InstallPlanError::UnsupportedVirtualAssets);
    }
    if catalog.map_to_resources == Some(true) {
        return Err(InstallPlanError::UnsupportedMappedAssets);
    }
    if !matches!(base.scheme(), "http" | "https") {
        return Err(InstallPlanError::UnsupportedUrlScheme {
            artifact: "asset object base".to_owned(),
            scheme: base.scheme().to_owned(),
        });
    }
    if base.query().is_some() || base.fragment().is_some() {
        return Err(InstallPlanError::InvalidAssetObjectBase);
    }

    let mut builder = InstallPlanBuilder::default();
    for object in catalog.objects.values() {
        let hash: Sha1Hash = object.hash.parse()?;
        let hash_text = hash.to_string();
        let url = asset_object_url(base, &hash_text)?;
        builder.push_file(PlannedFile {
            kind: PlannedFileKind::AssetObject,
            destination: layout.asset_object_path(hash),
            sources: vec![url],
            sha1: Some(hash),
            size: Some(object.size),
        })?;
    }
    Ok(builder.finish())
}

fn asset_object_url(base: &Url, hash: &str) -> Result<Url, InstallPlanError> {
    let mut url = base.clone();
    {
        let mut segments =
            url.path_segments_mut()
                .map_err(|_| InstallPlanError::UrlCannotBeBase {
                    artifact: format!("asset object {hash}"),
                    url: base.to_string(),
                })?;
        segments.pop_if_empty();
        segments.push(&hash[..2]);
        segments.push(hash);
    }
    Ok(url)
}

fn plan_library(
    layout: &MinecraftLayout,
    version_id: &VersionId,
    library: &Library,
    platform: &Platform,
    builder: &mut InstallPlanBuilder,
) -> Result<(), InstallPlanError> {
    if library.natives.is_empty() {
        let coordinate = MavenCoordinate::parse(&library.name)?;
        let download =
            match &library.downloads {
                Some(downloads) => Some(downloads.artifact.as_ref().ok_or_else(|| {
                    InstallPlanError::MissingLibraryArtifact(library.name.clone())
                })?),
                None => None,
            };
        return push_library_file(
            layout,
            library,
            &coordinate,
            download,
            PlannedFileKind::Library,
            builder,
        )
        .map(|_| ());
    }

    let Some(template) = library.natives.get(platform.os.as_str()) else {
        return Ok(());
    };
    let classifier = resolve_native_classifier(library, template, platform)?;
    let coordinate = MavenCoordinate::parse(&library.name)?.with_classifier(&classifier)?;
    let download = match &library.downloads {
        Some(downloads) => Some(downloads.classifiers.get(&classifier).ok_or_else(|| {
            InstallPlanError::MissingNativeClassifier {
                library: library.name.clone(),
                classifier: classifier.clone(),
            }
        })?),
        None => None,
    };

    let native_file = push_library_file(
        layout,
        library,
        &coordinate,
        download,
        PlannedFileKind::NativeLibrary,
        builder,
    )?;
    builder.push_extraction(ExtractionPlan {
        source: native_file,
        destination: layout.natives_dir(version_id, platform),
        exclude_prefixes: library
            .extract
            .as_ref()
            .map(|rules| rules.exclude.clone())
            .unwrap_or_default(),
    });
    Ok(())
}

fn push_library_file(
    layout: &MinecraftLayout,
    library: &Library,
    coordinate: &MavenCoordinate,
    download: Option<&DownloadInfo>,
    kind: PlannedFileKind,
    builder: &mut InstallPlanBuilder,
) -> Result<std::path::PathBuf, InstallPlanError> {
    let path = match download.and_then(|info| info.path.as_deref()) {
        Some(path) => RepositoryPath::new(path)?,
        None => coordinate.repository_path()?,
    };
    let destination = layout.library_path(&path);
    let url = match download.and_then(|info| info.url.as_deref()) {
        Some(url) => parse_url(url, &library.name)?,
        None => library_base_url(library.url.as_deref(), &path, &library.name)?,
    };
    let file = PlannedFile {
        kind,
        destination: destination.clone(),
        sources: vec![url],
        sha1: optional_sha1(download.and_then(|info| info.sha1.as_deref()))?,
        size: download.and_then(|info| info.size),
    };
    builder.push_file(file)?;
    Ok(destination)
}

fn resolve_native_classifier(
    library: &Library,
    template: &str,
    platform: &Platform,
) -> Result<String, InstallPlanError> {
    let classifier = if template.contains("${arch}") {
        let width = platform.arch.pointer_width().ok_or_else(|| {
            InstallPlanError::UnknownNativeArchitecture {
                library: library.name.clone(),
                classifier: template.to_owned(),
            }
        })?;
        template.replace("${arch}", &width.to_string())
    } else {
        template.to_owned()
    };

    if classifier.contains("${") {
        return Err(InstallPlanError::UnresolvedNativeClassifier {
            library: library.name.clone(),
            classifier,
        });
    }
    Ok(classifier)
}

fn library_base_url(
    base: Option<&str>,
    path: &RepositoryPath,
    artifact: &str,
) -> Result<Url, InstallPlanError> {
    let base = base.unwrap_or(DEFAULT_LIBRARY_BASE);
    let mut base_with_slash = base.to_owned();
    if !base_with_slash.ends_with('/') {
        base_with_slash.push('/');
    }
    let mut base_url = parse_url(&base_with_slash, artifact)?;
    {
        let mut segments =
            base_url
                .path_segments_mut()
                .map_err(|_| InstallPlanError::UrlCannotBeBase {
                    artifact: artifact.to_owned(),
                    url: base.to_owned(),
                })?;
        segments.pop_if_empty();
        for segment in path.as_str().split('/') {
            segments.push(segment);
        }
    }
    Ok(base_url)
}

fn required_url(value: Option<&str>, artifact: &str) -> Result<Url, InstallPlanError> {
    let value = value.ok_or_else(|| InstallPlanError::MissingDownloadUrl {
        artifact: artifact.to_owned(),
    })?;
    parse_url(value, artifact)
}

fn parse_url(value: &str, artifact: &str) -> Result<Url, InstallPlanError> {
    let url = Url::parse(value).map_err(|source| InstallPlanError::InvalidUrl {
        artifact: artifact.to_owned(),
        url: value.to_owned(),
        source,
    })?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(InstallPlanError::UnsupportedUrlScheme {
            artifact: artifact.to_owned(),
            scheme: url.scheme().to_owned(),
        });
    }
    Ok(url)
}

fn optional_sha1(value: Option<&str>) -> Result<Option<Sha1Hash>, InstallPlanError> {
    Ok(value.map(str::parse).transpose()?)
}
