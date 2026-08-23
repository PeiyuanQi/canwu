use super::{
    CanwuError, ErrorCode, RunConfiguration, RunConfigurationSnapshot, Scenario, canonical_hash,
    is_canonical_hash, policy,
};
use serde::{Deserialize, Serialize};

pub const RUN_MANIFEST_FORMAT_VERSION: u32 = 1;

/// Stable identity for a scenario, ruleset, content pack, run policy,
/// localization contract, or source ledger.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct ArtifactManifest {
    pub namespace: String,
    pub name: String,
    pub version: String,
    pub semantic_hash: String,
}

impl ArtifactManifest {
    pub fn new(
        namespace: impl Into<String>,
        name: impl Into<String>,
        version: impl Into<String>,
        semantic_hash: impl Into<String>,
    ) -> Result<Self, CanwuError> {
        let manifest = Self {
            namespace: namespace.into(),
            name: name.into(),
            version: version.into(),
            semantic_hash: semantic_hash.into(),
        };
        validate_artifact(&manifest, "artifact")?;
        Ok(manifest)
    }

    pub fn from_bytes(
        namespace: impl Into<String>,
        name: impl Into<String>,
        version: impl Into<String>,
        bytes: &[u8],
    ) -> Result<Self, CanwuError> {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"canwu.artifact-bytes.v1");
        hasher.update(&[0]);
        hasher.update(bytes);
        Self::new(
            namespace,
            name,
            version,
            hasher.finalize().to_hex().to_string(),
        )
    }

    pub fn for_scenario(
        namespace: impl Into<String>,
        name: impl Into<String>,
        version: impl Into<String>,
        scenario: &Scenario,
    ) -> Result<Self, CanwuError> {
        Self::new(namespace, name, version, scenario_semantic_hash(scenario)?)
    }

    pub fn for_run_configuration(
        namespace: impl Into<String>,
        name: impl Into<String>,
        version: impl Into<String>,
        configuration: &RunConfiguration,
    ) -> Result<Self, CanwuError> {
        let mut configuration = configuration.clone();
        configuration.canonicalize();
        configuration.validate()?;
        Self::new(namespace, name, version, configuration.semantic_hash()?)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "provenance", rename_all = "snake_case")]
/// The exact non-executable environment bound to a simulation run.
pub enum RunManifest {
    Declared {
        format_version: u32,
        scenario: ArtifactManifest,
        #[serde(default)]
        rules: Vec<ArtifactManifest>,
        #[serde(default)]
        content: Vec<ArtifactManifest>,
        #[serde(default)]
        localization_contracts: Vec<ArtifactManifest>,
        run_configuration: Box<ArtifactManifest>,
        #[serde(default)]
        sources: Vec<ArtifactManifest>,
    },
    MigratedLegacy {
        format_version: u32,
        source_engine_version: String,
        source_snapshot_format: u32,
        checkpoint_hash: String,
    },
}

impl RunManifest {
    pub fn for_scenario(
        namespace: impl Into<String>,
        name: impl Into<String>,
        version: impl Into<String>,
        scenario: &Scenario,
    ) -> Result<Self, CanwuError> {
        let scenario = ArtifactManifest::for_scenario(namespace, name, version, scenario)?;
        let run_configuration = default_run_configuration_manifest()?;
        Ok(Self::Declared {
            format_version: RUN_MANIFEST_FORMAT_VERSION,
            scenario,
            rules: Vec::new(),
            content: Vec::new(),
            localization_contracts: Vec::new(),
            run_configuration: Box::new(run_configuration),
            sources: Vec::new(),
        })
    }

    #[must_use]
    pub fn declared(scenario: ArtifactManifest, run_configuration: ArtifactManifest) -> Self {
        Self::Declared {
            format_version: RUN_MANIFEST_FORMAT_VERSION,
            scenario,
            rules: Vec::new(),
            content: Vec::new(),
            localization_contracts: Vec::new(),
            run_configuration: Box::new(run_configuration),
            sources: Vec::new(),
        }
    }

    pub(crate) fn migrated_legacy(
        source_engine_version: String,
        source_snapshot_format: u32,
        checkpoint_hash: String,
    ) -> Self {
        Self::MigratedLegacy {
            format_version: RUN_MANIFEST_FORMAT_VERSION,
            source_engine_version,
            source_snapshot_format,
            checkpoint_hash,
        }
    }
}

pub(crate) fn canonicalize(manifest: &mut RunManifest) {
    if let RunManifest::Declared {
        rules,
        content,
        localization_contracts,
        sources,
        ..
    } = manifest
    {
        rules.sort();
        content.sort();
        localization_contracts.sort();
        sources.sort();
    }
}

pub(crate) fn validate(
    manifest: &RunManifest,
    scenario: Option<&Scenario>,
    allow_legacy: bool,
) -> Result<(), CanwuError> {
    match manifest {
        RunManifest::Declared {
            format_version,
            scenario: scenario_manifest,
            rules,
            content,
            localization_contracts,
            run_configuration,
            sources,
        } => {
            if *format_version != RUN_MANIFEST_FORMAT_VERSION {
                return invalid_manifest(format!(
                    "run manifest format {format_version} is unsupported"
                ));
            }
            validate_artifact(scenario_manifest, "scenario")?;
            validate_artifact(run_configuration, "run configuration")?;
            validate_artifact_list(rules, "rules")?;
            validate_artifact_list(content, "content")?;
            validate_artifact_list(localization_contracts, "localization contract")?;
            validate_artifact_list(sources, "source")?;
            if let Some(scenario) = scenario
                && scenario_manifest.semantic_hash != scenario_semantic_hash(scenario)?
            {
                return invalid_manifest(
                    "scenario manifest hash does not match the admitted scenario",
                );
            }
            Ok(())
        }
        RunManifest::MigratedLegacy {
            format_version,
            source_engine_version,
            source_snapshot_format,
            checkpoint_hash,
        } => {
            if !allow_legacy {
                return invalid_manifest(
                    "new simulations require a declared scenario and run configuration manifest",
                );
            }
            if *format_version != RUN_MANIFEST_FORMAT_VERSION
                || source_engine_version.trim().is_empty()
                || source_engine_version != source_engine_version.trim()
                || !matches!(source_snapshot_format, 2 | 3)
                || !is_canonical_hash(checkpoint_hash)
            {
                return invalid_manifest("migrated legacy run provenance is invalid");
            }
            Ok(())
        }
    }
}

pub(crate) fn hash(manifest: &RunManifest) -> Result<String, CanwuError> {
    canonical_hash("canwu.run-manifest.v1", manifest)
}

pub(crate) fn validate_run_configuration(
    manifest: &RunManifest,
    configuration: &RunConfigurationSnapshot,
) -> Result<(), CanwuError> {
    configuration.validate()?;
    match (manifest, configuration) {
        (
            RunManifest::Declared {
                run_configuration, ..
            },
            RunConfigurationSnapshot::Declared(_) | RunConfigurationSnapshot::CompatibilityV1,
        ) => {
            if configuration.semantic_hash()?.as_deref()
                != Some(run_configuration.semantic_hash.as_str())
            {
                return invalid_manifest(
                    "run configuration snapshot does not match its manifest identity",
                );
            }
            Ok(())
        }
        (
            RunManifest::Declared {
                run_configuration, ..
            },
            RunConfigurationSnapshot::ManifestOnlyV1,
        ) => {
            if run_configuration.semantic_hash == policy::compatibility_configuration_hash()? {
                return invalid_manifest(
                    "the default run configuration must use compatibility-v1 provenance",
                );
            }
            Ok(())
        }
        (RunManifest::MigratedLegacy { .. }, RunConfigurationSnapshot::LegacyUnspecified) => Ok(()),
        (RunManifest::Declared { .. }, RunConfigurationSnapshot::LegacyUnspecified) => {
            invalid_manifest("declared runs cannot use an identity-unbound run configuration")
        }
        (RunManifest::MigratedLegacy { .. }, _) => invalid_manifest(
            "legacy migrations cannot silently inherit a current run configuration",
        ),
    }
}

fn scenario_semantic_hash(scenario: &Scenario) -> Result<String, CanwuError> {
    let mut canonical = scenario.clone();
    canonical.world.people.sort_by_key(|value| value.id);
    canonical.world.governments.sort_by_key(|value| value.id);
    canonical.world.territories.sort_by_key(|value| value.id);
    canonical.world.routes.sort_by_key(|value| value.id);
    canonical.world.armies.sort_by_key(|value| value.id);
    canonical
        .domain_records
        .sort_by(|left, right| left.reference.cmp(&right.reference));
    canonical_hash("canwu.scenario.v1", &canonical)
}

fn default_run_configuration_manifest() -> Result<ArtifactManifest, CanwuError> {
    ArtifactManifest::new(
        "canwu.core",
        "default-run-configuration",
        "1",
        policy::compatibility_configuration_hash()?,
    )
}

fn validate_artifact_list(manifests: &[ArtifactManifest], label: &str) -> Result<(), CanwuError> {
    let mut previous = None;
    for manifest in manifests {
        validate_artifact(manifest, label)?;
        if previous.is_some_and(|value: &ArtifactManifest| {
            value >= manifest
                || (value.namespace == manifest.namespace && value.name == manifest.name)
        }) {
            return invalid_manifest(format!(
                "{label} manifests must have unique identities and canonical order"
            ));
        }
        previous = Some(manifest);
    }
    Ok(())
}

fn validate_artifact(manifest: &ArtifactManifest, label: &str) -> Result<(), CanwuError> {
    if !canonical_text(&manifest.namespace)
        || !canonical_text(&manifest.name)
        || !canonical_text(&manifest.version)
        || !is_canonical_hash(&manifest.semantic_hash)
    {
        return invalid_manifest(format!(
            "{label} manifests require canonical namespace, name, version, and semantic hash"
        ));
    }
    Ok(())
}

fn canonical_text(value: &str) -> bool {
    !value.is_empty() && value == value.trim()
}

fn invalid_manifest<T>(message: impl Into<String>) -> Result<T, CanwuError> {
    Err(CanwuError::new(ErrorCode::InvalidRunManifest, message))
}
