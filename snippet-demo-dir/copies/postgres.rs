//! Implements [`super::Database`] using Postgres.

use analysis_dependency::{
    AnalysisDependenciesResponse, DependencyInstanceRow, DependencyInstances,
};
use async_trait::async_trait;
use auth::{ObjectIdentifier, RawToken, SuperToken, TemporaryCoreToken};
use color_eyre::eyre::{OptionExt, eyre};
use color_eyre::{Result, Section, eyre::Context};
use ext::{eyre::FutureContext, tokio::TryFutureTimeout};
use fingerprint::{Fingerprint, FingerprintContent, FingerprintKind, FlatSelectors, MatchRelation};
use futures_lite::StreamExt;
use itertools::Itertools;
use locator::StrictLocator;
use secrecy::ExposeSecret;
use sqlx::{Executor, PgPool, Postgres, postgres::PgPoolOptions};
use std::{collections::HashSet, time::Duration};
use strum::{AsRefStr, EnumString};
use thiserror::Error;
use tracing::{debug, info};

use super::*;

pub mod analysis_dependency;
pub mod fingerprint;
pub mod job;
pub mod queue;
pub mod vulnerability;
pub mod vulnerability_review;

mod timestamp;

/// Indicates whether a row was inserted or already existed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, EnumString, AsRefStr)]
enum RowStatus {
    #[strum(serialize = "existing")]
    #[default]
    Existing,
    #[strum(serialize = "inserted")]
    Inserted,
}

/// The migrator for the schema relied upon by this module.
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

/// Describes when a value could not be parsed because it was out of range for the base type.
#[derive(Debug, Error)]
#[error("value {0} is out of range for the type")]
pub struct RangeError<T>(T);

/// Defines converting to a base type for the given wrapper type.
///
/// This is used instead of having the type implement `sqlx::Encode`
/// because it doesn't work super well with the `query!` macro
/// (instead requiring `as T` overrides, removing the whole point of compile time query checks).
pub trait EncodePostgres {
    /// The base type that represents this value in the database.
    type Base;

    /// Convert to the base value used in the database.
    fn to_db(self) -> Self::Base;
}

/// Defines converting to from a base type to the given wrapper type.
///
/// This is used instead of having the type implement `sqlx::Decode`
/// because it doesn't work super well with the `query!` macro
/// (instead requiring `as T` overrides, removing the whole point of compile time query checks).
pub trait DecodePostgres {
    /// The base type that represents this value in the database.
    type Base;

    /// Construct a new instance from the base value used in the database.
    fn from_db(value: Self::Base) -> Self;
}

/// Defines fallibly converting from a base type to the given wrapper type.
///
/// This is used instead of having the type implement `sqlx::Decode`
/// because it doesn't work super well with the `query!` macro
/// (instead requiring `as T` overrides, removing the whole point of compile time query checks).
pub trait TryDecodePostgres {
    /// The base type that represents this value in the database.
    type Base;

    /// The error returned.
    type Err;

    /// Convert from the base value used in the database.
    fn try_from_db(value: Self::Base) -> Result<Self, Self::Err>
    where
        Self: std::marker::Sized;
}

/// Concrete implementation for an external postgres database.
#[derive(Clone)]
pub struct Pool {
    pool: PgPool,
}

impl Pool {
    /// Create a new instance connecting to the provided URL.
    #[deprecated = "DB operations and associated types in `client` are deprecated; prefer the `db` crate. See its docs for details."]
    pub async fn connect(url: impl Into<String>) -> Result<Self> {
        let url = url.into();

        // The number of connections weren't chosen for a particular reason,
        // they just seem reasonable.
        let pool = PgPoolOptions::new()
            .idle_timeout(Some(Duration::from_secs(30)))
            .max_connections(50)
            .min_connections(5)
            .connect(&url)
            .await
            .context("connect to database")?;

        // Migrations lock: https://docs.rs/sqlx/latest/sqlx/migrate/struct.Migrator.html#method.set_locking
        // This means that it is safe to migrate in the face of multiple clients
        // attempting to start simultaneously.
        crate::db::postgres::MIGRATOR
            .run(&pool)
            .await
            .context("migrate database")?;

        Ok(Self { pool })
    }

    /// Unwrap the pool into its underlying connection.
    pub fn into_inner(self) -> PgPool {
        self.pool
    }
}

impl std::fmt::Display for Pool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let conn = self.pool.connect_options();
        write!(
            f,
            "{}/{}",
            conn.get_host(),
            conn.get_database().unwrap_or("unknown")
        )
    }
}

impl std::fmt::Debug for Pool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self}")
    }
}

impl From<PgPool> for Pool {
    fn from(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl crate::db::Health for Pool {
    #[tracing::instrument]
    async fn healthcheck(&self) -> Result<()> {
        sqlx::query("select 1 as result")
            .fetch_one(&self.pool)
            .try_timeout(Duration::from_secs(1))
            .await
            .context("run query")
            .map(drop)
    }
}

#[async_trait]
impl super::Database for Pool {
    #[tracing::instrument]
    async fn match_find(&self, select: &MatchSelector) -> Result<HashSet<MatchRelation>> {
        // In the future we may add audit logs for the token here,
        // but for now it's discarded.
        find(self, &[select]).await
    }

    #[tracing::instrument]
    async fn match_list(&self, select: &[MatchSelector]) -> Result<HashSet<MatchRelation>> {
        // In the future we may add audit logs for the token here,
        // but for now it's discarded.
        find(self, select).await
    }

    #[tracing::instrument]
    async fn match_remove(&self, token: &SuperToken, select: &[MatchSelector]) -> Result<()> {
        let select = FlatSelectors::flatten(select);

        // There are three kinds of data to remove:
        // - Relations (this only removes the relation, not the underlying fingerprint/locator data)
        // - Fingerprint (this removes the fingerprint, cascading to relations referencing it)
        // - Locator (this removes the locator, cascading to relations referencing it)
        //
        // Since these are fundamentally different kinds of deletions on different tables,
        // they are run as separate queries in series inside a transaction.
        let mut tx = self.pool.begin().await.context("begin transaction")?;

        // First, delete the underlying data types: fingerprints...
        crate::db::postgres::remove_fingerprints(tx.as_mut(), &select)
            .await
            .context("delete fingerprints")?;

        // ... and locators.
        crate::db::postgres::remove_locators(tx.as_mut(), &select)
            .await
            .context("delete locators")?;

        // Then we can delete any relationships too.
        // These don't cascade to the underlying data types.
        crate::db::postgres::remove_relations(tx.as_mut(), &select)
            .await
            .context("delete relations")?;

        tx.commit().await.context("commit transaction")
    }

    #[tracing::instrument]
    async fn match_store(&self, token: &SuperToken, create: &[MatchRelation]) -> Result<()> {
        // First we create the api key if it doesn't exist,
        // which allows us to reference it in the rest of the transactions easily.
        crate::db::postgres::upsert_api_key(&self.pool, token)
            .await
            .context("upsert token")?;

        // Then insert the parts.
        let (locators, fps): (Vec<_>, Vec<_>) = create
            .iter()
            .map(|rel| (rel.locator(), rel.fingerprint()))
            .unzip();
        tokio::try_join!(
            crate::db::postgres::upsert_locators(&self.pool, locators, token)
                .then_context("upsert locators"),
            crate::db::postgres::upsert_fingerprints(&self.pool, fps, token)
                .then_context("upsert fingerprints")
        )?;

        // Now that the parts exist in the database, we can safely create relations.
        crate::db::postgres::upsert_relations(&self.pool, create, token)
            .await
            .context("upsert relations")?;

        Ok(())
    }

    /// Create a temporary token referencing a token from Core.
    #[tracing::instrument(skip(core_token))]
    async fn create_temporary_token(
        &self,
        core_token: &RawToken,
        object_identifier: &ObjectIdentifier,
    ) -> Result<TemporaryCoreToken> {
        let new_token = TemporaryCoreToken::generate();

        sqlx::query!(
            r#"
            insert into signed_url_token (token, core_auth_token, object_identifier)
            values ($1, $2, $3)
            "#,
            new_token.expose_secret(),
            core_token.expose_secret(),
            object_identifier.inner()
        )
        .execute(&self.pool)
        .await
        .context("Store signed url token")?;

        Ok(new_token)
    }

    /// Check that a token in the database exists and is not expired.
    #[tracing::instrument(skip_all)]
    async fn fetch_core_token(&self, token: TemporaryCoreToken) -> Result<RawToken> {
        let inner_token = token.expose_secret();

        let rec = sqlx::query!(
            r#"
            delete from signed_url_token
            where token = $1
            returning core_auth_token, object_identifier, created_at
            "#,
            inner_token
        )
        .fetch_one(&self.pool)
        .await
        .context("Fetch and delete one-time-use signed url token")?;

        let max_token_ttl = time::Duration::minutes(5);

        if time::OffsetDateTime::now_utc() - rec.created_at > max_token_ttl {
            return Err(eyre!("Token creation time is > {}", max_token_ttl));
        }

        Ok(RawToken::new(rec.core_auth_token))
    }

    /// Given an analysis and its dependencies, create the analysis and dependency_instances and associated join tables
    ///
    /// We de-duplicate dependency_instances in two ways.
    /// First, if the hash of the full dependency graph is the same as an existing dependency graph, then we can re-use the existing dependency graph.
    /// If this happens, we will create an analysis pointing to the existing dependency_list_hash and then return.
    /// If the hash of the dependency graph does not already exist in the DB, then we re-use existing dependency instances and create any
    /// dependency_instances that do not exist. This re-use is the second kind of de-duplication.
    /// The final step is to join all of the dependency instances, whether new or pre-existing, to the dependency_list_hash via the dependency_list_hash_dependency_instance
    /// join table.
    #[tracing::instrument(fields(dependencies_requested = dependencies.len(), dependencies_created, dependency_list_hashes_created))]
    async fn insert_analysis_dependencies(
        &self,
        organization_id: i64,
        root_locator: String,
        dependencies: Vec<DependencyInstance>,
    ) -> Result<i64> {
        // We are creating these rows in a few separate queries, so we run it in a transaction
        let mut tx = self.pool.begin().await.context("begin transaction")?;
        // Create the dependency_list_hash entry in the DB. If it already exists, then we know that the dependency instances already exist,
        // so we just need to create the analysis entry and then leave.
        // If it does not exist, then we need to create the `dependency_instance`s and the `dependency_list_hash_dependency_instance`
        // entries that join them to the dependency_list_hash.
        let analysis_hash = DependencyInstances::from(dependencies.clone()).hash();
        let dependency_list_hash_row = sqlx::query!(
            r#"
              WITH
              extant AS (
                  SELECT id, 'existing' as status FROM analysis_dependency.dependency_list_hash WHERE dependency_list_hash = ($1)
              ),
              inserted AS (
                  INSERT INTO analysis_dependency.dependency_list_hash (dependency_list_hash)
                  SELECT $1
                  WHERE NOT EXISTS (SELECT FROM extant)
                  RETURNING id, 'inserted' as status
              )
              SELECT id, status FROM inserted
              UNION ALL
              SELECT id, status FROM extant
            "#,
            analysis_hash,
        )
        .fetch_one(tx.as_mut()).await.with_context(|| format!("inserting new dependency_list_hash for hash {analysis_hash:?}"))?;
        let dependency_list_hash_id = dependency_list_hash_row
            .id
            .ok_or_eyre("getting id when creating dependency_list_hash")?;
        let dependency_list_hash_status = dependency_list_hash_row
            .status
            .map(|s| s.parse::<RowStatus>().unwrap_or_default())
            .ok_or_eyre("getting insert status when creating dependency_list_hash")?;

        let analysis_id_row = sqlx::query!(
            r#"
              INSERT INTO analysis_dependency.analysis
              (organization_id, root_locator, dependency_list_hash_id)
              VALUES ($1, $2, $3)
              RETURNING id
            "#,
            organization_id,
            root_locator,
            dependency_list_hash_id,
        )
        .fetch_one(tx.as_mut())
        .await
        .context("creating new analysis")?;
        let analysis_id = analysis_id_row.id;

        // If the dependency_list_hash already exists, then all of the dependency_instance and dependency_list_hash_dependency_instance entries also exist
        // and we can just return early.
        if dependency_list_hash_status == RowStatus::Existing {
            tx.commit().await.context("commit transaction")?;
            tracing::Span::current().record("dependencies_created", 0);
            tracing::Span::current().record("dependency_list_hashes_created", 0);
            info!(
                telemetry.dependencies_requested = dependencies.len(),
                telemetry.dependencies_created = 0,
                telemetry.dependency_list_hashes_created = 0,
            );

            return Ok(analysis_id);
        }

        // If we're here, then we created a new dependency_list_hash, so we need to create new dependency instances and
        // the dependency_list_hash_dependency_instance join tables.
        // Some or all of the dependency instances may already exist. That's fine. For ones that already exist, we
        // just create the entry in dependency_list_hash_dependency_instance.
        //
        // The common upsert trick (overwrite on conflict) is non-performant, and we need this to be performant.
        // Reference: https://stackoverflow.com/a/42217872/1046689
        let (
            hashes,
            locators,
            resolved,
            unresolved_locators,
            origin_paths,
            depth,
            manual,
            paths_to,
        ): (
            Vec<Vec<u8>>,
            Vec<String>,
            Vec<bool>,
            Vec<Result<serde_json::Value, serde_json::Error>>,
            Vec<Result<serde_json::Value, serde_json::Error>>,
            Vec<i64>,
            Vec<bool>,
            Vec<Result<serde_json::Value, serde_json::Error>>,
        ) = dependencies
            .iter()
            .map(|d| {
                (
                    d.hash(),
                    d.locator.to_string(),
                    d.resolved,
                    serde_json::to_value(d.unresolved_locators.clone()),
                    serde_json::to_value(d.origin_paths.clone()),
                    d.depth,
                    d.manual,
                    serde_json::to_value(d.paths_to.clone()),
                )
            })
            .multiunzip();
        let origin_paths = origin_paths.into_iter().collect::<Result<Vec<_>, _>>()?;
        let paths_to = paths_to.into_iter().collect::<Result<Vec<_>, _>>()?;
        let unresolved_locators = unresolved_locators
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?;
        // Create new dependency_instances and get the IDs of already existing dependency_instances.
        let dependency_instances = sqlx::query!(
            r#"
            with input_rows (hash, locator, resolved, manual, depth) as (
                select * from unnest ($1::bytea[], $2::text[], $3::bool[], $4::jsonb[], $5::bool[], $6::bigint[], $7::jsonb[], $8::jsonb[])
            ),
            ins as (
                insert into analysis_dependency.dependency_instance
                (hash, locator, resolved, unresolved_locators, manual, depth, paths_to, origin_paths)
                select * from input_rows
                on conflict do nothing
                RETURNING id, 'inserted' as status
            )
            SELECT id, status
            FROM   ins
            UNION  ALL
            SELECT c.id, 'existing' as status
            FROM   input_rows
            JOIN   analysis_dependency.dependency_instance c USING (hash);
            "#,
            &hashes,
            &locators,
            &resolved,
            &unresolved_locators,
            &manual,
            &depth,
            &paths_to,
            &origin_paths,
        )
        .fetch_all(tx.as_mut())
        .await
        .context("creating dependency instances")?;

        let (dependency_instance_ids, dependency_list_hash_ids): (Vec<Option<i64>>, Vec<i64>) =
            dependency_instances
                .iter()
                .map(|d| (d.id, dependency_list_hash_id))
                .multiunzip();
        let dependency_instance_ids = dependency_instance_ids
            .into_iter()
            .collect::<Option<Vec<i64>>>()
            .ok_or_eyre("getting list of created dependency instances")?;

        // The last step is to create the dependency_list_hash_dependency_instance rows to join the dependency_instances to the dependency_list_hash
        sqlx::query!(
            r#"
            with input_rows (dependency_instance_id, dependency_list_hash_id) as (
                select * from unnest ($1::bigint[], $2::bigint[])
            ),
            ins as (
                insert into analysis_dependency.dependency_list_hash_dependency_instance
                (dependency_instance_id, dependency_list_hash_id)
                select * from input_rows
                on conflict do nothing
                returning dependency_list_hash_id, dependency_instance_id
            )
            select
                (dependency_list_hash_id, dependency_instance_id) from ins
                union all
                    select (c.dependency_list_hash_id, c.dependency_instance_id) from input_rows
                    JOIN
                    analysis_dependency.dependency_list_hash_dependency_instance c
                        using (dependency_list_hash_id, dependency_instance_id)
            "#,
            &dependency_instance_ids,
            &dependency_list_hash_ids,
        )
        .fetch_all(tx.as_mut())
        .await
        .context("creating dependency_list_hash_dependency_instances")?;

        let created_instances = dependency_instances
            .iter()
            .filter(|di| {
                di.status.as_ref().is_some_and(|s| {
                    s.parse::<RowStatus>().unwrap_or_default() == RowStatus::Inserted
                })
            })
            .collect::<Vec<_>>();

        tracing::Span::current().record("dependencies_created", created_instances.len());
        tracing::Span::current().record("dependency_list_hashes_created", 1);
        info!(
            dependencies_requested = dependencies.len(),
            dependencies_created = created_instances.len(),
            dependency_list_hashes_created = 1,
        );
        tx.commit().await.context("commit transaction")?;
        Ok(analysis_id)
    }

    #[tracing::instrument]
    async fn get_analysis_dependencies(
        &self,
        analysis_id: i64,
        organization_id: i64,
        filters: &DependencyFilters,
    ) -> Result<AnalysisDependenciesResponse, Error> {
        // This query is a bit complex. The idea of the UNION is to always have a row returned if the analysis exists,
        // even if there are no dependencies or all of the dependencies are filtered out.
        // We'll get three different types of responses to this query:
        // 1. The analysis does not exist. In this case, we get no rows back from the query.
        // 2. The analysis exists, but there are no dependencies or all of the dependencies are filtered out.
        //    In this case, we get one row back from the query, and that row will have sentinel set to true.
        // 3. The analysis exists and there are dependencies. In this case, we get the dependency rows back from the query. There will still be a row with sentinel set to true, and we need to filter that out before converting to DependencyInstances.
        //
        // We can't just use a LEFT JOIN here because we need to return a row even if all of the dependency_instance rows are filtered out.
        let dependency_rows = sqlx::query_as!(
            DependencyInstanceRow,
            r#"
            select
              false as sentinel,
              di.locator,
              di.resolved,
              di.unresolved_locators,
              di.origin_paths,
              di.manual,
              di.depth,
              di.paths_to
              from analysis_dependency.analysis a
              inner join analysis_dependency.dependency_list_hash_dependency_instance dlhdi
                on dlhdi.dependency_list_hash_id = a.dependency_list_hash_id
              inner join analysis_dependency.dependency_instance di
                on di.id = dlhdi.dependency_instance_id
              where a.organization_id = $1 and a.id = $2 and
                case $3::boolean when true then (di.depth <= $4::bigint OR di.depth IS NULL) else true end and
                case $5::boolean when true then (split_part(di.locator, '+', 1) = any($6::text[]) OR di.locator IS NULL) else true end and
                case $7::boolean when true then (di.depth >= $8::bigint OR di.depth IS NULL) else true end
              UNION select
                true as sentinel,
                '' as locator,
                false as resolved,
                '{}' as unresolved_locators,
                '{}' as origin_paths,
                false as manual,
                0 as depth,
                '{}' as paths_to
              from analysis_dependency.analysis a where a.id = $2 AND a.organization_id = $1;"#,
            organization_id,
            analysis_id,
            filters.max_depth.is_some(),
            filters.max_depth,
            filters.fetchers.is_some(),
            filters.fetchers.as_deref(),
            filters.min_depth.is_some(),
            filters.min_depth
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::Transport(eyre!("error fetching dependency instances: {:?}", e)))?;

        match dependency_rows.as_slice() {
            // No rows returned, so the analysis does not exist.
            [] => Err(Error::NotFound),
            // One row returned, and sentinel is true, so there are no dependencies.
            [
                DependencyInstanceRow {
                    sentinel: Some(true),
                    ..
                },
            ] => Ok(AnalysisDependenciesResponse {
                dependencies: Vec::new(),
            }),
            // One row returned, and sentinel is None, so there are dependencies.
            // This shouldn't happen, but guard against it anyways.
            [DependencyInstanceRow { sentinel: None, .. }] => Ok(AnalysisDependenciesResponse {
                dependencies: Vec::new(),
            }),
            // Multiple rows returned, so there are dependencies.
            // Filter out the sentinel row and convert the rest to DependencyInstances.
            _ => {
                let dependencies = dependency_rows
                    .into_iter()
                    .filter(|row| !row.sentinel.unwrap_or(true))
                    .map(|row| row.try_into())
                    .collect::<Result<Vec<_>, Error>>()?;
                Ok(AnalysisDependenciesResponse { dependencies })
            }
        }
    }
}

async fn find(db: &Pool, select: &[impl AsRef<MatchSelector>]) -> Result<HashSet<MatchRelation>> {
    if select.is_empty() {
        return Ok(HashSet::new());
    }

    // Previously, we looked up everything in one big query, but this involves a lot of joins
    // and is overall a lot of load on the database.
    //
    // Instead we split into up to three smaller and more focused queries and union in the service.
    // This is better for a couple reasons:
    // 1. It allows the service to avoid sending the more complex query when a simpler query will do.
    //    For example, if we're only looking up fingerprints, all the extra joins and unions are wasted effort.
    // 2. Joining the results in the service is comparatively much easier than in the DB.
    //    The DB is more general purpose, so it has higher overhead for e.g. materializing views of unioned data.
    //    Meanwhile we can easily deduplicate into a hashset without any major cost.
    //
    // These are cloned because they're going into spawned futures, which need ownership of the values.
    let (fingerprints, locators, relations) = select.iter().fold(
        (Vec::new(), Vec::new(), Vec::new()),
        |(mut fingerprints, mut locators, mut relations), sel| {
            match sel.as_ref() {
                MatchSelector::Relation(relation) => relations.push(relation.clone()),
                MatchSelector::Locator(locator) => locators.push(locator.clone()),
                MatchSelector::Fingerprint(fingerprint) => fingerprints.push(fingerprint.clone()),
            }
            (fingerprints, locators, relations)
        },
    );

    // Clones the DB pool since spawned futures need ownership of the value.
    // Clones of the DB pool are cheap.
    let db = db.clone();

    // There's a lot of checking for empty lists in this section of the code,
    // but doing it once more here allows us to avoid even the overhead of spawning the future
    // when we know nothing needs to be looked up.
    let mut handles = Vec::new();
    if !locators.is_empty() {
        handles.push(tokio::spawn(find_locator(db.clone(), locators)));
    }
    if !fingerprints.is_empty() {
        handles.push(tokio::spawn(find_fingerprint(db.clone(), fingerprints)));
    }
    if !relations.is_empty() {
        handles.push(tokio::spawn(find_relation(db.clone(), relations)));
    }

    // This looks like we're running the operations serially,
    // but actually they're running in parallel
    // since they're spawned into the runtime.
    let mut combined = HashSet::new();
    for handle in handles {
        let subset = handle
            .await
            .context("join task")?
            .context("query database")?;
        combined.extend(subset);
    }

    Ok(combined)
}

async fn find_fingerprint(db: Pool, select: Vec<Fingerprint>) -> Result<HashSet<MatchRelation>> {
    if select.is_empty() {
        return Ok(HashSet::new());
    }

    let select = FlatSelectors::from(select);

    sqlx::query!(
        r#"
        with search_fingerprints (kind, content) as (
            select * from unnest( $1::text[], $2::bytea[] )
        )
        select
            fingerprint.kind,
            fingerprint.content,
            locator.content as locator
        from search_fingerprints
        join match.fingerprint
            on fingerprint.kind = search_fingerprints.kind
            and fingerprint.content = search_fingerprints.content
        join match.relation on relation.fingerprint = fingerprint.id
        join match.locator on locator.id = relation.locator
        "#,
        &select.fp_kind(),
        &select.fp_content(),
    )
    .fetch(&db.pool)
    .map(|row| -> Result<MatchRelation> {
        let row = row.context("read row")?;

        let locator = StrictLocator::parse(&row.locator)
            .with_context(|| format!("parse locator: '{}'", row.locator))?;
        let kind = FingerprintKind::new(row.kind);
        let content = FingerprintContent::new(row.content);
        let fingerprint = Fingerprint::new(kind, content);

        Ok(MatchRelation::new(locator, fingerprint))
    })
    .try_collect()
    .await
}

async fn find_locator(db: Pool, select: Vec<StrictLocator>) -> Result<HashSet<MatchRelation>> {
    if select.is_empty() {
        return Ok(HashSet::new());
    }

    let select = FlatSelectors::from(select);
    sqlx::query!(
        r#"
        with search_locators (locator) as (
            select * from unnest( $1::text[] )
        )
        select
            fingerprint.kind,
            fingerprint.content,
            locator.content as locator
        from search_locators
        join match.locator on locator.content = search_locators.locator
        join match.relation on relation.locator = locator.id
        join match.fingerprint on fingerprint.id = relation.fingerprint
        "#,
        &select.locator(),
    )
    .fetch(&db.pool)
    .map(|row| -> Result<MatchRelation> {
        let row = row.context("read row")?;

        let locator = StrictLocator::parse(&row.locator)
            .with_context(|| format!("parse locator: '{}'", row.locator))?;
        let kind = FingerprintKind::new(row.kind);
        let content = FingerprintContent::new(row.content);
        let fingerprint = Fingerprint::new(kind, content);

        Ok(MatchRelation::new(locator, fingerprint))
    })
    .try_collect()
    .await
}

async fn find_relation(db: Pool, select: Vec<MatchRelation>) -> Result<HashSet<MatchRelation>> {
    if select.is_empty() {
        return Ok(HashSet::new());
    }

    let select = FlatSelectors::from(select);
    sqlx::query!(
        r#"
        with search_relations (kind, content, locator) as (
            select * from unnest(
                $1::text[],
                $2::bytea[],
                $3::text[]
            )
        )
        select
            fingerprint.kind,
            fingerprint.content,
            locator.content as locator
        from search_relations
        join match.fingerprint
            on fingerprint.kind = search_relations.kind
            and fingerprint.content = search_relations.content
        join match.locator on locator.content = search_relations.locator
        join match.relation
            on relation.locator = locator.id
            and relation.fingerprint = fingerprint.id
        "#,
        &select.rel_kind(),
        &select.rel_content(),
        &select.rel_locator(),
    )
    .fetch(&db.pool)
    .map(|row| -> Result<MatchRelation> {
        let row = row.context("read row")?;

        let locator = StrictLocator::parse(&row.locator)
            .with_context(|| format!("parse locator: '{}'", row.locator))?;
        let kind = FingerprintKind::new(row.kind);
        let content = FingerprintContent::new(row.content);
        let fingerprint = Fingerprint::new(kind, content);

        Ok(MatchRelation::new(locator, fingerprint))
    })
    .try_collect()
    .await
}

/// Upsert api key row entry into the database.
pub async fn upsert_api_key<'a, E>(db: E, auth: &SuperToken) -> Result<()>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query!(
        r#"
        insert into match.api_key (content)
        values ($1)
        on conflict (content) do nothing
        "#,
        &auth.expose_secret(),
    )
    .execute(db)
    .await
    .context("run query")
    .map(drop)
}

/// Upsert `Locator` rows into the database.
pub async fn upsert_locators<'a, E>(
    db: E,
    locators: impl IntoIterator<Item = &StrictLocator>,
    auth: &SuperToken,
) -> Result<()>
where
    E: Executor<'a, Database = Postgres>,
{
    let locators = locators
        .into_iter()
        .map(|loc| loc.to_string())
        .collect::<Vec<_>>();

    // The common upsert trick (overwrite on conflict) is non-performant, and we need this to be performant.
    // Reference: https://stackoverflow.com/a/42217872/1046689
    sqlx::query!(
        r#"
        with input_rows (content, api_key) as (
            select *, $2 from unnest ( $1::text[] )
        ),
        keyed_input_rows (content, created_by) as (
            select input_rows.content, api_key.id
            from input_rows
            join match.api_key on input_rows.api_key = api_key.content
        )
        insert into match.locator (content, created_by)
        select content, created_by from keyed_input_rows
        on conflict (content) do nothing
        "#,
        &locators,
        &auth.expose_secret(),
    )
    .execute(db)
    .await
    .context("run query")
    .map(drop)
}

/// Upsert `Fingerprint` rows into the database.
pub async fn upsert_fingerprints<'a, E>(
    db: E,
    fingerprints: impl IntoIterator<Item = &Fingerprint>,
    auth: &SuperToken,
) -> Result<()>
where
    E: Executor<'a, Database = Postgres>,
{
    let (kinds, contents): (Vec<_>, Vec<_>) = fingerprints
        .into_iter()
        .map(|fp| {
            (
                fp.kind().as_ref().to_owned(),
                fp.content().as_ref().to_owned(),
            )
        })
        .unzip();

    // The common upsert trick (overwrite on conflict) is non-performant, and we need this to be performant.
    // Reference: https://stackoverflow.com/a/42217872/1046689
    sqlx::query!(
        r#"
        with input_rows (kind, content, api_key) as (
            select *, $3 from unnest ( $1::text[], $2::bytea[] )
        ),
        keyed_input_rows (kind, content, created_by) as (
            select input_rows.kind, input_rows.content, api_key.id
            from input_rows
            join match.api_key on input_rows.api_key = api_key.content
        )
        insert into match.fingerprint (kind, content, created_by)
        select kind, content, created_by from keyed_input_rows
        on conflict (kind, content) do nothing
        "#,
        &kinds,
        &contents,
        &auth.expose_secret(),
    )
    .execute(db)
    .await
    .context("run query")
    .map(drop)
}

/// Upsert [`Relation`] rows into the database.
pub async fn upsert_relations<'a, E>(
    db: E,
    relations: impl IntoIterator<Item = &MatchRelation>,
    auth: &SuperToken,
) -> Result<()>
where
    E: Executor<'a, Database = Postgres>,
{
    let (locators, kinds, contents): (Vec<_>, Vec<_>, Vec<_>) = relations
        .into_iter()
        .map(|rel| {
            (
                rel.locator().to_string(),
                rel.fingerprint().kind().as_ref().to_string(),
                rel.fingerprint().content().as_ref().to_vec(),
            )
        })
        .multiunzip();

    // The common upsert trick (overwrite on conflict) is non-performant, and we need this to be performant.
    // Reference: https://stackoverflow.com/a/42217872/1046689
    sqlx::query!(
        r#"
        with input_rows (locator, kind, content, api_key) as (
            select *, $4 from unnest ( $1::text[], $2::text[], $3::bytea[] )
        ),
        keyed_input_rows (locator, fingerprint, created_by) as (
            select locator.id, fingerprint.id, api_key.id
            from input_rows
            join match.api_key on input_rows.api_key = api_key.content
            join match.locator on input_rows.locator = locator.content
            join match.fingerprint
                on input_rows.kind = fingerprint.kind
                and input_rows.content = fingerprint.content
        )
        insert into match.relation (locator, fingerprint, created_by)
        select locator, fingerprint, created_by from keyed_input_rows
        on conflict (locator, fingerprint) do nothing
        "#,
        &locators,
        &kinds,
        &contents,
        &auth.expose_secret(),
    )
    .execute(db)
    .await
    .context("run query")
    .map(drop)
}

/// Removes `Fingerprint` rows from database, cascading to relations referencing it.
async fn remove_fingerprints<'a, E>(db: E, select: &FlatSelectors) -> Result<()>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query!(
        r#"
        delete from match.fingerprint
                    where kind = any($1::text[])
                    and content = any($2::bytea[])
        "#,
        select.fp_kind(),
        select.fp_content(),
    )
    .execute(db)
    .await
    .inspect(|res| debug!("deleted fingerprints: {}", res.rows_affected()))
    .context("run query")
    .map(drop)
}

/// Removes `Locator` rows from databases, cascading to relations referencing it.
async fn remove_locators<'a, E>(db: E, select: &FlatSelectors) -> Result<()>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query!(
        r#"
        delete from match.locator
                    where content = any($1::text[])
        "#,
        select.locator(),
    )
    .execute(db)
    .await
    .inspect(|res| debug!("deleted locators: {}", res.rows_affected()))
    .context("run query")
    .map(drop)
}

/// Removes `Relation` rows from databases. This only removes the relation, not the underlying fingerprint/locator data.
async fn remove_relations<'a, E>(db: E, select: &FlatSelectors) -> Result<()>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query!(
        r#"with search (kind, content, locator) as (
            select * from unnest(
                $1::text[],
                $2::bytea[],
                $3::text[]
            )
        ),
        target_relations (fingerprint, locator) as (
            select fingerprint.id, locator.id
            from match.relation
            join match.locator on locator.id = relation.locator
            join match.fingerprint on fingerprint.id = relation.fingerprint
            where exists(
                select 1 from search
                where search.locator = locator.content
                and search.kind = fingerprint.kind
                and search.content = fingerprint.content
            )
        )
        delete from match.relation
        where (fingerprint, locator) in (
            select * from target_relations
        )"#,
        select.rel_kind(),
        select.rel_content(),
        select.rel_locator(),
    )
    .execute(db)
    .await
    .inspect(|res| debug!("deleted locators: {}", res.rows_affected()))
    .context("run query")
    .map(drop)
}
