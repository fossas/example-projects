//! Implements [`Database`] by wrapping an existing instance,
//! then extending some of its searches with Maven Central.

use std::collections::HashSet;

use crate::db::Error;
use crate::db::postgres::fingerprint::Fingerprint;
use crate::maven_central::{self, Identifier, Version};
use async_trait::async_trait;
use auth::{ObjectIdentifier, RawToken, SuperToken, TemporaryCoreToken};
use color_eyre::{Result, eyre::Context};
use itertools::{Either, Itertools};
use locator::StrictLocator;
use tap::Pipe;
use tracing::{debug, info};

use super::postgres::fingerprint::{MatchRelation, MatchSelector};
use super::{Database, DatabaseLayer};

/// Wraps another implementation of [`Database`],
/// extending it with Maven Central.
///
/// Specifically, it adds a new method that supports looking up
/// the JAR directly in Maven Central, and adds it to the database
/// if the JAR's other fingerprints aren't already in the DB.
#[derive(Clone)]
pub struct Wrapper {
    db: DatabaseLayer,
}

impl Wrapper {
    /// Wrap the underlying database.
    pub fn wrap(db: impl Database + 'static) -> Self {
        Self {
            db: DatabaseLayer::new(db),
        }
    }

    /// Attempt to look up the fingerprints for a specific JAR using the database; if not found:
    /// 1. Attempt to find the JAR in Maven Central.
    /// 2. If it's found, hydrate it into the db for future lookups.
    /// 3. Then return the locator.
    ///
    /// Importantly, _all the fingerprints provided should be for a single jar_.
    pub async fn jar_find(&self, jar: &[Fingerprint]) -> Result<HashSet<MatchRelation>> {
        jar_find(&self.db, jar).await
    }
}

impl std::fmt::Display for Wrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MavenCentralDb({:?})", self.db)
    }
}

impl std::fmt::Debug for Wrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self}")
    }
}

#[async_trait]
impl crate::db::Health for Wrapper {
    #[tracing::instrument]
    async fn healthcheck(&self) -> Result<()> {
        self.db.healthcheck().await
    }
}

#[async_trait]
impl super::Database for Wrapper {
    #[tracing::instrument(skip_all)]
    async fn match_find(&self, select: &MatchSelector) -> Result<HashSet<MatchRelation>> {
        if let Some(fingerprint) = select.as_fingerprint(fingerprint::Kind::JarMavenCentralV1) {
            jar_find(&self.db, [fingerprint]).await
        } else {
            self.db.match_find(select).await
        }
    }

    #[tracing::instrument(skip_all)]
    async fn match_list(&self, select: &[MatchSelector]) -> Result<HashSet<MatchRelation>> {
        let (fingerprints, rest): (Vec<&Fingerprint>, Vec<MatchSelector>) =
            select.iter().partition_map(|selector| {
                if let Some(fingerprint) =
                    selector.as_fingerprint(fingerprint::Kind::JarMavenCentralV1)
                {
                    Either::Left(fingerprint)
                } else {
                    Either::Right(selector.clone())
                }
            });

        tokio::try_join!(jar_find(&self.db, fingerprints), self.db.match_list(&rest)).map(
            |(mut left, right)| {
                left.extend(right);
                left
            },
        )
    }

    #[tracing::instrument(skip_all)]
    async fn match_remove(&self, token: &SuperToken, select: &[MatchSelector]) -> Result<()> {
        self.db.match_remove(token, select).await
    }

    #[tracing::instrument(skip_all)]
    async fn match_store(&self, token: &SuperToken, create: &[MatchRelation]) -> Result<()> {
        self.db.match_store(token, create).await
    }

    /// Create a new token for the given organization.
    #[tracing::instrument(skip_all)]
    async fn create_temporary_token(
        &self,
        core_token: &RawToken,
        object_identifier: &ObjectIdentifier,
    ) -> Result<TemporaryCoreToken> {
        self.db
            .create_temporary_token(core_token, object_identifier)
            .await
    }

    /// Check that a token in the database exists and is not expired.
    #[tracing::instrument(skip_all)]
    async fn fetch_core_token(&self, token: TemporaryCoreToken) -> Result<RawToken> {
        self.db.fetch_core_token(token).await
    }

    /// Given an analysis and its dependencies, create the analysis and dependency_instances and associated join tables
    async fn insert_analysis_dependencies(
        &self,
        organization_id: i64,
        root_locator: String,
        dependencies: Vec<crate::db::DependencyInstance>,
    ) -> Result<i64> {
        self.db
            .insert_analysis_dependencies(organization_id, root_locator, dependencies)
            .await
    }

    /// Given an analysis ID and an organization ID, find the associated dependencies
    async fn get_analysis_dependencies(
        &self,
        analysis_id: i64,
        organization_id: i64,
        filters: &super::DependencyFilters,
    ) -> Result<crate::db::AnalysisDependenciesResponse, Error> {
        self.db
            .get_analysis_dependencies(analysis_id, organization_id, filters)
            .await
    }
}

/// Attempt to look up the fingerprints for a specific JAR using the database; if not found:
/// 1. Attempt to find the JAR in Maven Central.
/// 2. If it's found, hydrate it into the db for future lookups.
/// 3. Then return the locator.
///
/// Importantly, _all the fingerprints provided should be for a single jar_.
#[tracing::instrument(skip(jar))]
pub async fn jar_find(
    db: &DatabaseLayer,
    jar: impl IntoIterator<Item = &Fingerprint>,
) -> Result<HashSet<MatchRelation>> {
    let jar = jar.into_iter().collect_vec();
    let selectors = jar
        .iter()
        .map(|&jar| jar.to_owned())
        .map(MatchSelector::Fingerprint)
        .collect::<Vec<_>>();

    let relations = db.match_list(&selectors).await.context("lookup matches")?;
    if !relations.is_empty() {
        info!("jar cache hit");
        return Ok(relations);
    }

    info!("jar cache miss");
    let Some(lookup) = jar.iter().find_map(|fp| {
        if fp.kind.as_ref() == fingerprint::Kind::JarMavenCentralV1.as_ref() {
            Some(hex::encode(fp.content.as_ref()))
        } else {
            None
        }
    }) else {
        return Ok(HashSet::new());
    };

    let Some(locator) = resolve_jar_locator_hash(&lookup)
        .await
        .context("lookup jar fingerprint")?
    else {
        return Ok(HashSet::new());
    };

    let relations = jar
        .iter()
        .map(|&jar| jar.to_owned())
        .map(|fingerprint| MatchRelation {
            fingerprint,
            locator: locator.clone(),
        })
        .collect::<Vec<_>>();
    db.match_store(SuperToken::internal(), &relations)
        .await
        .context("store match relations")?;

    relations.into_iter().collect::<HashSet<_>>().pipe(Ok)
}

/// Resolve the JAR at the provided path to a locator by the hash of its content.
#[tracing::instrument]
pub async fn resolve_jar_locator_hash(target: &str) -> Result<Option<StrictLocator>> {
    let matches = maven_central::search::shared()
        .search_jar_hash(target)
        .await
        .context("find matches for jar")?;
    if matches.is_empty() {
        return Ok(None);
    }

    // Maven central reports _all_ the JARs associated with the fingerprint,
    // even if it only actually matched one.
    // So we have to download and hash them the same way to see which matches.
    for matched in matches {
        debug!(%target, ?matched, "testing whether result matches expected jar");
        let id = Identifier::from(&matched);
        let version = Version::from(&matched);

        let matched_hash = maven_central::blob::shared()
            .sha1(&id, &version)
            .await
            .context("get sha1 of matched jar")?;
        if matched_hash == target {
            return StrictLocator::from(matched).pipe(Some).pipe(Ok);
        }
    }

    Ok(None)
}
