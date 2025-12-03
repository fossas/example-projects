//! Types and primitives for analysis_dependencies.
use crate::db::Error;
use color_eyre::eyre::eyre;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use typed_builder::TypedBuilder;

/// A list of lists of dependencies that lead to this dependency
/// E.g. if npm+commander$2.20.3 is a dependency of npm+d3-dev$2.0.0
/// and command is also a dependency of npm+editorconfig$0.15.3 which is a dependency of npm+js-beautify$1.10.3, then
/// PathsTo will look like
/// {"paths":[["npm+js-beautify$1.10.3","npm+editorconfig$0.15.3","npm+commander$2.20.3"],["npm+d3-dsv$2.0.0","npm+commander$2.20.3"]]}}
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Debug, Deserialize, Serialize)]
pub struct PathsTo(BTreeSet<Vec<String>>);

impl<I: IntoIterator<Item = Vec<String>>> From<I> for PathsTo {
    fn from(value: I) -> Self {
        Self(value.into_iter().collect())
    }
}

impl PathsTo {
    /// Get the inner paths
    pub fn inner(&self) -> &BTreeSet<Vec<String>> {
        &self.0
    }
}

/// A row from the dependency_instance table
#[derive(Clone, Eq, PartialEq, Debug, TypedBuilder, Serialize, Deserialize)]
pub struct DependencyInstanceRow {
    /// Whether the row is a sentinel row
    pub sentinel: Option<bool>,
    /// The locator of the Dependency. This is parsed as a locator in the API layer, but treated as an opaque string in the DB.
    pub locator: Option<String>,
    /// The resolution status
    pub resolved: Option<bool>,
    /// The original, unresolved locators that were in the build graph and resolved to the locator found in the locator field.
    /// These are parsed as locators in the API locator, but treated as opaque strings in the DB.
    pub unresolved_locators: Option<serde_json::Value>,
    /// Paths to the manifest files where this dependency was found
    pub origin_paths: Option<serde_json::Value>,
    /// The depth of the dependency
    pub depth: Option<i64>,
    /// Whether the dependency was manually created
    pub manual: Option<bool>,
    /// Paths (of dependencies) to this dependency
    pub paths_to: Option<serde_json::Value>,
}

impl TryFrom<DependencyInstanceRow> for DependencyInstance {
    type Error = Error;

    fn try_from(value: DependencyInstanceRow) -> Result<Self, Self::Error> {
        if value.sentinel.unwrap_or(true) {
            return Err(Error::Transport(eyre!("attempted to convert sentinel row")));
        }
        let locator = value
            .locator
            .ok_or(Error::Transport(eyre!("locator is null")))?;
        let resolved = value
            .resolved
            .ok_or(Error::Transport(eyre!("resolved is null")))?;
        let unresolved_locators = value
            .unresolved_locators
            .ok_or(Error::Transport(eyre!("unresolved_locators is null")))?;
        let unresolved_locators = serde_json::from_value(unresolved_locators).map_err(|e| {
            Error::Transport(eyre!("converting from unresolved_locators json: {}", e))
        })?;
        let origin_paths = value
            .origin_paths
            .ok_or(Error::Transport(eyre!("origin_paths is null")))?;
        let origin_paths = serde_json::from_value(origin_paths)
            .map_err(|e| Error::Transport(eyre!("converting from origin_paths json: {}", e)))?;
        let depth = value
            .depth
            .ok_or(Error::Transport(eyre!("depth is null")))?;
        let manual = value
            .manual
            .ok_or(Error::Transport(eyre!("manual is null")))?;
        let paths_to = value
            .paths_to
            .ok_or(Error::Transport(eyre!("paths_to is null")))?;
        let paths_to: PathsTo = serde_json::from_value(paths_to)
            .map_err(|e| Error::Transport(eyre!("converting from paths_to json: {}", e)))?;
        Ok(Self {
            locator,
            resolved,
            unresolved_locators,
            origin_paths,
            depth,
            manual,
            paths_to,
        })
    }
}

/// A Dependency of an analysis
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Debug, TypedBuilder, Serialize, Deserialize)]
pub struct DependencyInstance {
    /// The locator of the Dependency. This is parsed as a locator in the API layer, but treated as an opaque string in the DB.
    pub locator: String,
    /// The resolution status
    pub resolved: bool,
    /// The original, unresolved locators that were in the build graph and resolved to the locator found in the locator field.
    /// These are parsed as locators in the API locator, but treated as opaque strings in the DB.
    /// These are typically found by the CLI.
    /// There can be multiple unresolved locators if the same locator was resolved from multiple sources.
    /// E.g. if the latest version of underscore is 1.2.3 and we try to resolve "npm+underscore" and "npm+underscore$1.2.3",
    /// both will resolve to "npm+underscore$1.2.3".
    pub unresolved_locators: BTreeSet<String>,
    /// Paths to the manifest files where this dependency was found
    pub origin_paths: BTreeSet<String>,
    /// The depth of the dependency
    pub depth: i64,
    /// Whether the dependency was manually created
    pub manual: bool,
    /// Paths (of dependencies) to this dependency
    pub paths_to: PathsTo,
}

impl DependencyInstance {
    /// Generate a hash of the DependencyInstance as a string. Used for generating a hash for DependencyInstances
    pub fn hashable(&self) -> String {
        format!(
            "{}::{}::{:?}::{:?}::{}::{}::{:?}",
            self.locator,
            self.resolved,
            self.unresolved_locators,
            self.origin_paths,
            self.depth,
            self.manual,
            self.paths_to
        )
    }

    /// Generate a hash of the DependencyInstance as bytes
    pub fn hash(&self) -> Vec<u8> {
        let mut hasher = Sha256::new();
        hasher.update(self.hashable().as_bytes());
        hasher.finalize().to_vec()
    }
}

/// An ordered collection of DependencyInstance. Used for generating a hash for a list of DependencyInstance
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Default)]
pub struct DependencyInstances {
    inner: BTreeSet<DependencyInstance>,
}

impl DependencyInstances {
    /// Generate a hash of the DependencyInstances as bytes
    pub fn hash(&self) -> Vec<u8> {
        let hash_string = self
            .inner
            .iter()
            .map(|di| di.hashable())
            .collect::<Vec<_>>()
            .join("::");

        let mut hasher = Sha256::new();
        hasher.update(hash_string.as_bytes());
        hasher.finalize().to_vec()
    }
}

impl<I: IntoIterator<Item = DependencyInstance>> From<I> for DependencyInstances {
    fn from(value: I) -> Self {
        Self {
            inner: value.into_iter().collect(),
        }
    }
}

/// A response containing the dependencies of an analysis and the root locator of the analysis
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AnalysisDependenciesResponse {
    /// The dependencies of the analysis
    pub dependencies: Vec<DependencyInstance>,
}
