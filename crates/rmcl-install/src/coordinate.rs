use rmcl_instance::{LayoutError, RepositoryPath};
use thiserror::Error;

/// A Maven artifact coordinate used by Minecraft library metadata.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MavenCoordinate {
    group: String,
    artifact: String,
    version: String,
    classifier: Option<String>,
    extension: String,
}

impl MavenCoordinate {
    pub fn parse(input: &str) -> Result<Self, MavenCoordinateError> {
        let (coordinates, extension) = split_extension(input)?;
        let parts: Vec<_> = coordinates.split(':').collect();
        if !(3..=4).contains(&parts.len()) {
            return Err(invalid(
                input,
                "expected group:artifact:version[:classifier]",
            ));
        }
        if parts.iter().any(|part| !valid_part(part)) {
            return Err(invalid(input, "empty or unsafe coordinate component"));
        }
        if parts[0].split('.').any(str::is_empty) {
            return Err(invalid(input, "group contains an empty segment"));
        }

        Ok(Self {
            group: parts[0].to_owned(),
            artifact: parts[1].to_owned(),
            version: parts[2].to_owned(),
            classifier: parts.get(3).map(|classifier| (*classifier).to_owned()),
            extension,
        })
    }

    pub fn with_classifier(
        &self,
        classifier: impl Into<String>,
    ) -> Result<Self, MavenCoordinateError> {
        let classifier = classifier.into();
        if !valid_part(&classifier) {
            return Err(MavenCoordinateError::InvalidCoordinate {
                coordinate: classifier,
                reason: "empty or unsafe native classifier",
            });
        }
        let mut result = self.clone();
        result.classifier = Some(classifier);
        Ok(result)
    }

    pub fn file_name(&self) -> String {
        let classifier = self
            .classifier
            .as_deref()
            .map(|value| format!("-{value}"))
            .unwrap_or_default();
        format!(
            "{}-{}{}.{}",
            self.artifact, self.version, classifier, self.extension
        )
    }

    pub fn repository_path(&self) -> Result<RepositoryPath, LayoutError> {
        RepositoryPath::new(format!(
            "{}/{}/{}/{}",
            self.group.replace('.', "/"),
            self.artifact,
            self.version,
            self.file_name()
        ))
    }
}

fn split_extension(input: &str) -> Result<(&str, String), MavenCoordinateError> {
    let (coordinates, extension) = match input.split_once('@') {
        Some((coordinates, extension)) => {
            if coordinates.contains('@') || !valid_extension(extension) {
                return Err(invalid(input, "invalid artifact extension"));
            }
            (coordinates, extension.to_owned())
        }
        None => (input, "jar".to_owned()),
    };
    Ok((coordinates, extension))
}

fn valid_part(part: &str) -> bool {
    !part.is_empty()
        && part != "."
        && part != ".."
        && !part.contains(['/', '\\', '@'])
        && !part.chars().any(char::is_whitespace)
        && !part.chars().any(char::is_control)
}

fn valid_extension(extension: &str) -> bool {
    valid_part(extension) && !extension.starts_with('.')
}

fn invalid(coordinate: &str, reason: &'static str) -> MavenCoordinateError {
    MavenCoordinateError::InvalidCoordinate {
        coordinate: coordinate.to_owned(),
        reason,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MavenCoordinateError {
    #[error("invalid Maven coordinate `{coordinate}`: {reason}")]
    InvalidCoordinate {
        coordinate: String,
        reason: &'static str,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_standard_maven_path() {
        let coordinate = MavenCoordinate::parse("com.example:demo-library:1.2.3").unwrap();
        assert_eq!(coordinate.file_name(), "demo-library-1.2.3.jar");
        assert_eq!(
            coordinate.repository_path().unwrap().as_str(),
            "com/example/demo-library/1.2.3/demo-library-1.2.3.jar"
        );
    }

    #[test]
    fn supports_classifier_and_extension() {
        let coordinate =
            MavenCoordinate::parse("org.example:native-lib:2.0:natives-linux@zip").unwrap();
        assert_eq!(
            coordinate.repository_path().unwrap().as_str(),
            "org/example/native-lib/2.0/native-lib-2.0-natives-linux.zip"
        );
    }

    #[test]
    fn replaces_classifier_for_native_artifacts() {
        let coordinate = MavenCoordinate::parse("org.example:native-lib:2.0")
            .unwrap()
            .with_classifier("natives-windows")
            .unwrap();
        assert_eq!(coordinate.file_name(), "native-lib-2.0-natives-windows.jar");
    }

    #[test]
    fn rejects_malformed_coordinates() {
        for input in [
            "",
            "group:artifact",
            "group:artifact:version:one:two",
            "group::version",
            "group..example:artifact:version",
            "group:artifact:../version",
            "group:artifact:version/extra",
            "group:artifact:version@",
            "group:artifact:version@.jar",
            "group:artifact:version@zip@other",
            "group:artifact:ver sion",
        ] {
            assert!(MavenCoordinate::parse(input).is_err(), "accepted {input:?}");
        }
    }
}
