//! Implements [`Database`] by wrapping an existing instance,
//! then extending some of its searches with lookups to the
//! primary instance of Sparkle, if it is configured.

use super::{
    Database, DatabaseLayer, MatchRelation, MatchSelector, ObjectIdentifier, TemporaryCoreToken,
};
use crate::db::Error;
use crate::sparkle::SparkleLayer;
use async_trait::async_trait;
use auth::{RawToken, SuperToken};
use color_eyre::Result;
use std::collections::HashSet;

/// Wraps an implementation
pub struct Wrapper {
    db: DatabaseLayer,
    primary: Option<SparkleLayer>,
}

impl Wrapper {
    /// Wrap the underyling database.
    pub fn wrap(db: impl Database + 'static, primary: Option<SparkleLayer>) -> Self {
        Self {
            db: DatabaseLayer::new(db),
            primary,
        }
    }
}

impl std::fmt::Display for Wrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PrimarySparkleDb({:?})", self.db)
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
impl Database for Wrapper {
    #[tracing::instrument(skip_all)]
    async fn match_find(&self, select: &MatchSelector) -> Result<HashSet<MatchRelation>> {
        self.db.match_find(select).await
    }

    #[tracing::instrument(skip_all)]
    async fn match_list(&self, select: &[MatchSelector]) -> Result<HashSet<MatchRelation>> {
        match &self.primary {
            Some(client) => client
                .fingerprint_list(select.iter().map(|ms| ms.to_owned().into()).collect())
                .await
                .map(|relations| relations.into_iter().map(MatchRelation::from).collect()),
            None => self.db.match_list(select).await,
        }
    }

    #[tracing::instrument(skip_all)]
    async fn match_remove(&self, token: &SuperToken, select: &[MatchSelector]) -> Result<()> {
        self.db.match_remove(token, select).await
    }

    #[tracing::instrument(skip_all)]
    async fn match_store(&self, token: &SuperToken, create: &[MatchRelation]) -> Result<()> {
        self.db.match_store(token, create).await
    }

    /// Create a new token for the given core token and opaque object_identifier.
    async fn create_temporary_token(
        &self,
        core_token: &RawToken,
        object_identifier: &ObjectIdentifier,
    ) -> Result<TemporaryCoreToken> {
        self.db
            .create_temporary_token(core_token, object_identifier)
            .await
    }

    /// Check that a token in the database exists, is not expired, and return its associated core token.
    ///
    /// Performing this check means that the SignedUrlToken is no longer valid since it can be used only once.
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
