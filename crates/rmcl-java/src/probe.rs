use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};

use rmcl_domain::Architecture;
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::{Child, Command},
};

use crate::{JavaError, JavaRuntime, JavaRuntimeSource};

#[derive(Debug, Clone)]
pub struct ProbeConfig {
    pub timeout: Duration,
    pub max_output: usize,
}

impl Default for ProbeConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(5),
            max_output: 64 * 1024,
        }
    }
}

pub async fn probe(
    executable: impl Into<PathBuf>,
    source: JavaRuntimeSource,
) -> Result<JavaRuntime, JavaError> {
    probe_with_config(executable, source, ProbeConfig::default()).await
}

pub async fn probe_with_config(
    executable: impl Into<PathBuf>,
    source: JavaRuntimeSource,
    config: ProbeConfig,
) -> Result<JavaRuntime, JavaError> {
    let executable = executable.into();
    if !std::fs::metadata(&executable)
        .map(|metadata| metadata.is_file())
        .unwrap_or(false)
    {
        return Err(JavaError::NotFound(executable.display().to_string()));
    }

    let mut command = Command::new(&executable);
    command
        .arg("-XshowSettings:properties")
        .arg("-version")
        .env("LC_ALL", "C")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn().map_err(|source| JavaError::Spawn {
        path: executable.display().to_string(),
        source,
    })?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| JavaError::MissingOutput {
            path: executable.display().to_string(),
            stream: "stdout",
        })?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| JavaError::MissingOutput {
            path: executable.display().to_string(),
            stream: "stderr",
        })?;

    let probe = async {
        let path = executable.display().to_string();
        let status = async {
            child
                .wait()
                .await
                .map_err(|source| io_error(&executable, source))
        };
        let stdout = read_limited(stdout, config.max_output, &executable);
        let stderr = read_limited(stderr, config.max_output, &executable);
        let (status, _, stderr) = tokio::try_join!(status, stdout, stderr)?;
        if !status.success() {
            return Err(JavaError::ProbeFailed {
                path,
                status: status.to_string(),
                output: String::from_utf8_lossy(&stderr).trim().to_owned(),
            });
        }
        parse_runtime(executable.clone(), source, &stderr)
    };

    match tokio::time::timeout(config.timeout, probe).await {
        Ok(Ok(runtime)) => Ok(runtime),
        Ok(Err(error)) => {
            terminate(&mut child).await;
            Err(error)
        }
        Err(_) => {
            terminate(&mut child).await;
            Err(JavaError::ProbeTimeout(executable.display().to_string()))
        }
    }
}

pub fn parse_major_version(version: &str) -> Option<u32> {
    let mut parts = version.trim().split('.');
    let first = parts.next()?;
    if first == "1" {
        return parts.next()?.parse().ok();
    }
    let numeric_prefix: String = first.chars().take_while(char::is_ascii_digit).collect();
    if numeric_prefix.is_empty() {
        None
    } else {
        numeric_prefix.parse().ok()
    }
}

fn parse_runtime(
    executable: PathBuf,
    source: JavaRuntimeSource,
    output: &[u8],
) -> Result<JavaRuntime, JavaError> {
    let text = String::from_utf8_lossy(output);
    let properties = parse_properties(&text);
    let path = executable.display().to_string();
    let version =
        properties
            .get("java.version")
            .cloned()
            .ok_or_else(|| JavaError::MissingProperty {
                path: path.clone(),
                field: "java.version",
            })?;
    let major_version = parse_major_version(&version).ok_or_else(|| JavaError::InvalidVersion {
        path,
        version: version.clone(),
    })?;
    let architecture = properties
        .get("os.arch")
        .map(|value| Architecture::from_name(value))
        .ok_or_else(|| JavaError::MissingProperty {
            path: executable.display().to_string(),
            field: "os.arch",
        })?;

    Ok(JavaRuntime {
        executable,
        version,
        major_version,
        architecture,
        vendor: properties.get("java.vendor").cloned(),
        source,
    })
}

fn parse_properties(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            let key = key.trim();
            if key.is_empty() {
                None
            } else {
                Some((key.to_owned(), value.trim().to_owned()))
            }
        })
        .collect()
}

async fn read_limited(
    reader: impl AsyncRead + Unpin,
    limit: usize,
    executable: &Path,
) -> Result<Vec<u8>, JavaError> {
    let mut limited = reader.take((limit as u64).saturating_add(1));
    let mut output = Vec::new();
    limited
        .read_to_end(&mut output)
        .await
        .map_err(|source| io_error(executable, source))?;
    if output.len() > limit {
        Err(JavaError::OutputTooLarge(executable.display().to_string()))
    } else {
        Ok(output)
    }
}

async fn terminate(child: &mut Child) {
    let _ = child.kill().await;
    let _ = child.wait().await;
}

fn io_error(path: &Path, source: std::io::Error) -> JavaError {
    JavaError::Io {
        path: path.display().to_string(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_modern_and_legacy_major_versions() {
        for (version, expected) in [
            ("1.8.0_202", Some(8)),
            ("1.9.0", Some(9)),
            ("17.0.11", Some(17)),
            ("21", Some(21)),
            ("21-ea", Some(21)),
            ("not-a-version", None),
            ("", None),
        ] {
            assert_eq!(parse_major_version(version), expected);
        }
    }

    #[test]
    fn parses_properties_with_whitespace_and_duplicates() {
        let properties = parse_properties(
            "  java.version = 17.0.1\n os.arch = amd64\njava.vendor = First\njava.vendor = Second\ninvalid line\n",
        );
        assert_eq!(properties["java.version"], "17.0.1");
        assert_eq!(properties["os.arch"], "amd64");
        assert_eq!(properties["java.vendor"], "Second");
        assert!(!properties.contains_key("invalid line"));
    }

    #[cfg(unix)]
    fn write_script(content: &str) -> (tempfile::TempDir, PathBuf) {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::TempDir::new().unwrap();
        let path = directory.path().join("java");
        std::fs::write(&path, content).unwrap();
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).unwrap();
        (directory, path)
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn probes_a_real_process() {
        let (_directory, executable) = write_script(
            "#!/bin/sh\nprintf '    java.version = 17.0.11\n    os.arch = amd64\n    java.vendor = TestVendor\n' >&2\n",
        );

        let runtime = probe(executable, JavaRuntimeSource::Custom).await.unwrap();

        assert_eq!(runtime.major_version, 17);
        assert_eq!(runtime.architecture, Architecture::X86_64);
        assert_eq!(runtime.vendor.as_deref(), Some("TestVendor"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn kills_a_probe_that_times_out() {
        let (_directory, executable) = write_script("#!/bin/sh\nsleep 60\n");

        let error = probe_with_config(
            executable,
            JavaRuntimeSource::Custom,
            ProbeConfig {
                timeout: Duration::from_millis(50),
                ..ProbeConfig::default()
            },
        )
        .await
        .unwrap_err();

        assert!(matches!(error, JavaError::ProbeTimeout(_)));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rejects_excessive_probe_output() {
        let (_directory, executable) = write_script("#!/bin/sh\nyes x\n");

        let error = probe_with_config(
            executable,
            JavaRuntimeSource::Custom,
            ProbeConfig {
                timeout: Duration::from_secs(2),
                max_output: 32,
            },
        )
        .await
        .unwrap_err();

        assert!(matches!(error, JavaError::OutputTooLarge(_)));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn reports_failed_probe_processes() {
        let (_directory, executable) = write_script("#!/bin/sh\necho failure >&2\nexit 7\n");

        let error = probe(executable, JavaRuntimeSource::Custom)
            .await
            .unwrap_err();

        assert!(matches!(error, JavaError::ProbeFailed { .. }));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rejects_missing_java_properties() {
        let (_directory, executable) = write_script("#!/bin/sh\nexit 0\n");

        let error = probe(executable, JavaRuntimeSource::Custom)
            .await
            .unwrap_err();

        assert!(matches!(error, JavaError::MissingProperty { .. }));
    }
}
