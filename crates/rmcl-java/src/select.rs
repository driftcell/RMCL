use crate::{JavaRequirement, JavaRuntime};

/// Selects the lowest compatible runtime, preferring managed installations at equal versions.
pub fn select_java<'a>(
    runtimes: &'a [JavaRuntime],
    requirement: &JavaRequirement,
) -> Option<&'a JavaRuntime> {
    runtimes
        .iter()
        .filter(|runtime| requirement.accepts(runtime))
        .min_by_key(|runtime| (runtime.major_version, runtime.source))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::JavaRuntimeSource;

    fn runtime(major: u32, source: JavaRuntimeSource) -> JavaRuntime {
        JavaRuntime {
            executable: format!("java-{major}").into(),
            version: major.to_string(),
            major_version: major,
            operating_system: rmcl_domain::OperatingSystem::Linux,
            architecture: rmcl_domain::Architecture::X86_64,
            vendor: None,
            source,
        }
    }

    #[test]
    fn declared_major_is_a_minimum_and_newer_is_allowed() {
        let requirement = JavaRequirement {
            component: None,
            major_version: Some(17),
            maximum_major: None,
            operating_system: rmcl_domain::OperatingSystem::Linux,
            architecture: rmcl_domain::Architecture::X86_64,
        };
        let runtimes = [
            runtime(21, JavaRuntimeSource::Managed),
            runtime(17, JavaRuntimeSource::System),
        ];

        assert!(requirement.accepts(&runtimes[0]));
        assert!(requirement.accepts(&runtimes[1]));
        assert_eq!(
            select_java(&runtimes, &requirement).unwrap().major_version,
            17
        );

        let only_newer = [runtime(21, JavaRuntimeSource::System)];
        assert_eq!(
            select_java(&only_newer, &requirement)
                .unwrap()
                .major_version,
            21
        );
    }

    #[test]
    fn missing_major_uses_java_8_as_the_legacy_baseline() {
        let requirement = JavaRequirement {
            component: None,
            major_version: None,
            maximum_major: None,
            operating_system: rmcl_domain::OperatingSystem::Linux,
            architecture: rmcl_domain::Architecture::X86_64,
        };
        let runtimes = [
            runtime(21, JavaRuntimeSource::Managed),
            runtime(8, JavaRuntimeSource::System),
            runtime(17, JavaRuntimeSource::Managed),
        ];

        assert_eq!(
            select_java(&runtimes, &requirement).unwrap().major_version,
            8
        );
    }

    #[test]
    fn managed_runtime_wins_at_the_same_major() {
        let requirement = JavaRequirement {
            component: None,
            major_version: Some(17),
            maximum_major: None,
            operating_system: rmcl_domain::OperatingSystem::Linux,
            architecture: rmcl_domain::Architecture::X86_64,
        };
        let runtimes = [
            runtime(17, JavaRuntimeSource::System),
            runtime(17, JavaRuntimeSource::Managed),
        ];

        assert_eq!(
            select_java(&runtimes, &requirement).unwrap().source,
            JavaRuntimeSource::Managed
        );
    }
}
