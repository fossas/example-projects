//! A job queue implemented with Postgres.

#![allow(
    clippy::needless_lifetimes,
    reason = "False positive in `Assignment`, statement level allow doesn't fix"
)]

use std::time::Duration;

use crate::db::postgres::{
    DecodePostgres, EncodePostgres, TryDecodePostgres,
    job::{JobId, JobState},
};
use bon::{Builder, builder};
use color_eyre::eyre::{Context, Report, Result, eyre};
use derivative::Derivative;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sqlx::{Executor, Pool, Postgres, Transaction};
use strum::{EnumIter, FromRepr};
use tap::Pipe;
use time::OffsetDateTime;
use tokio::time::interval;

/// Whether a failed job will be retried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failed {
    /// The failure was final.
    Final,

    /// The job will be retried after a delay.
    Retry,
}

/// Available job queues.
#[derive(
    Copy,
    Clone,
    Eq,
    PartialEq,
    PartialOrd,
    Ord,
    Hash,
    Debug,
    Serialize,
    Deserialize,
    FromRepr,
    EnumIter,
)]
pub enum Queue {
    /// The queue to handle reachability jobs.
    Reachability = 1,

    /// The queue to handle discovery jobs.
    Discovery,

    /// The queue to handle maven fingerprint ingestion jobs.
    IngestFingerprintsMaven,

    /// The queue to handle caching jobs.
    Caching,

    /// The queue to handle NVD scrape jobs.
    ScrapeNvd,

    /// The queue to handle pypa/python advisory DB scrape jobs.
    ScrapePythonAdvisory,

    /// The queue to handle CodeSentry scrape jobs.
    ScrapeCodeSentry,

    /// The queue to handle PHP Advisories scrape jobs.
    ScrapePhpAdvisories,

    /// The queue to handle Anchore scrap jobs.
    ScrapeAnchore,

    /// The queue to handle ruby advisory DB scrape jobs.
    ScrapeRubyAdvisory,

    /// The queue to handle Victims CVE DB scrape jobs.
    ScrapeVictims,

    /// The queue to handle retire js scrape jobs.
    ScrapeRetireJs,

    /// The queue to handle CISA KEV scrape jobs.
    ScrapeCisa,

    /// The queue to handle EPSS scrape jobs.
    ScrapeEpss,

    /// The queue to handle rustsec advisory DB scrape jobs.
    ScrapeRustSecAdvisory,

    ///The queue to handle GHSA scrape jobs.
    CrawlGhsa,

    /// The queue to handle temp storage cleanup jobs.
    TempStorageCleanup,
}

impl Queue {
    /// Iterate through the variants.
    pub fn iter() -> QueueIter {
        <Queue as strum::IntoEnumIterator>::iter()
    }
}

impl EncodePostgres for Queue {
    type Base = i32;

    fn to_db(self) -> Self::Base {
        self as i32
    }
}

impl TryDecodePostgres for Queue {
    type Base = i32;
    type Err = Report;

    fn try_from_db(value: Self::Base) -> Result<Self, Self::Err>
    where
        Self: std::marker::Sized,
    {
        match Queue::from_repr(value as usize) {
            Some(value) => Ok(value),
            None => Err(eyre!("{value} is out of range for this type")),
        }
    }
}

impl IntoIterator for Queue {
    type Item = Self;
    type IntoIter = std::iter::Once<Self>;

    fn into_iter(self) -> Self::IntoIter {
        std::iter::once(self)
    }
}

impl From<&Queue> for Queue {
    fn from(value: &Queue) -> Self {
        *value
    }
}

/// A job that is to be inserted in the queue.
#[derive(Debug, Builder)]
pub struct Request<T> {
    /// The queue on which to place the job.
    pub queue: Queue,

    /// The data associated to the job.
    pub data: T,

    /// The number of retries for this job.
    ///
    /// If not specified, defaults to 3.
    /// If higher than [`i32::MAX`], clamped to that max value.
    #[builder(into)]
    pub max_retries: Option<usize>,

    /// The time in the future to perform the job.
    /// If not specified, the job is scheduled immediately.
    #[builder(into)]
    pub scheduled: Option<OffsetDateTime>,
}

impl<T: Serialize> Request<T> {
    /// Put a job on the queue to be worked.
    pub async fn enqueue<'a, E: Executor<'a, Database = Postgres>>(&self, db: E) -> Result<JobId> {
        // If the user specifies a scheduled time, don't pass that time to the database directly:
        // this is because the database may have a different time than we do.
        // Instead, get the offset of the user's desired date and pass that. Postgres then uses the same offset.
        let offset_seconds = self
            .scheduled
            .map(|scheduled| {
                let difference = scheduled - OffsetDateTime::now_utc();
                let offset = difference.as_seconds_f64();
                offset.max(0.)
            })
            .unwrap_or_default();

        let max_retries = self.max_retries.unwrap_or(3).min(i32::MAX as usize) as i32;
        sqlx::query!(
            "
            insert into job (queue, data, state, max_retries, scheduled_at)
            values ($1, $2, $3, $4, now() + make_interval(secs => $5))
            returning id
            ",
            self.queue.to_db(),
            bincode::serialize(&self.data).context("serialize data")?,
            JobState::Pending.to_db(),
            max_retries,
            offset_seconds,
        )
        .fetch_one(db)
        .await
        .context("enqueue job")
        .map(|result| JobId::from_db(result.id))
    }
}

/// A job that has been assigned from the queue.
///
/// This job contains a database transaction that has reserved the job.
/// Dropping this job without marking it complete rolls back the transaction
/// and makes this same job available for other workers.
///
/// That being said, this is not the ideal way to handle errors by default,
/// because doing so doesn't advance the retry and can't record errors.
#[derive(Derivative)]
#[derivative(Debug)]
pub struct Assignment<'a> {
    #[derivative(Debug = "ignore")]
    tx: Transaction<'a, Postgres>,
    data: Vec<u8>,

    /// The ID of the job.
    pub id: i64,

    /// The queue the job belongs to.
    pub queue: Queue,

    /// The number of times this job may be retried.
    pub max_retries: i32,

    /// The number of times this job has failed.
    pub retries: i32,

    /// Each time the job fails, the error is appended to this list.
    /// If this job has had retries, previous errors can be accessed here.
    pub failures: Vec<String>,
}

impl<'a> Assignment<'a> {
    /// Get a job from one of the given queues to be worked.
    ///
    /// This function works by opening a transaction and reserving a job,
    /// then keeping it open while the job is worked on (this transaction is actually embedded in the returned job).
    ///
    /// If the job is dropped before it is marked complete, the transaction is rolled back.
    /// If no jobs are available when this function is run, it returns `None`.
    ///
    /// This function is cancellation safe.
    pub async fn try_accept(
        db: &Pool<Postgres>,
        queues: impl IntoIterator<Item = impl Into<Queue>>,
    ) -> Result<Option<Self>> {
        let mut tx = db.begin().await.context("begin transaction")?;
        let queues = queues
            .into_iter()
            .map(Into::into)
            .map(|q| q as i32)
            .collect::<Vec<_>>();

        sqlx::query!(
            "
            update job
            set started = now()
            where id = (
                select id from job
                where scheduled_at <= now() and started is null and finished is null and queue = any($1)
                order by id for update skip locked limit 1
            )
            returning id, queue, data, retries, max_retries, failures
            ",
            &queues
        )
        .fetch_optional(tx.as_mut())
        .await
        .context("reserve job")?
        .map(move |job| Assignment {
            tx,
            queue: Queue::try_from_db(job.queue).expect("valid queue"),
            id: job.id,
            data: job.data,
            max_retries: job.max_retries,
            retries: job.retries.expect("define job retries"),
            failures: job.failures,
        })
        .pipe(Ok)
    }

    /// Get a job from one of the given queues to be worked.
    ///
    /// This function is like [`Self::try_accept`], except that if no jobs are available when this
    /// function is run it continually polls the database until a job is available.
    ///
    /// This function is cancellation safe.
    pub async fn accept(
        db: &Pool<Postgres>,
        queues: impl IntoIterator<Item = impl Into<Queue>>,
    ) -> Result<Self> {
        let mut ticker = interval(Duration::from_secs(1));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let queues = queues.into_iter().map(Into::into).collect::<Vec<_>>();

        loop {
            if let Some(job) = Self::try_accept(db, &queues).await? {
                break Ok(job);
            }

            ticker.tick().await;
        }
    }

    /// Decode the job data as the specified type.
    pub fn data<T: DeserializeOwned>(&self) -> Result<T> {
        bincode::deserialize(&self.data).context("deserialize data")
    }

    /// Mark the job completed successfully.
    pub async fn complete(mut self) -> Result<()> {
        sqlx::query!(
            "update job set state = $1, finished = now() where id = $2",
            JobState::Completed.to_db(),
            self.id
        )
        .execute(self.tx.as_mut())
        .await
        .context("mark job complete")?;

        self.tx.commit().await.context("commit transaction")
    }

    /// Mark the job failed with the provided error.
    pub async fn fail_with(self, err: &Report) -> Result<Failed> {
        self.fail(format!("{err:#}")).await
    }

    /// Mark the job failed with the provided error.
    pub async fn fail(mut self, err: impl Into<String>) -> Result<Failed> {
        let failed = if self.retries >= self.max_retries {
            sqlx::query!(
                "
                update job set
                    started = null,
                    finished = now(),
                    state = $2,
                    failures = array_append(failures, $3)
                where id = $1
                ",
                self.id,
                JobState::Failed.to_db(),
                err.into(),
            )
            .execute(self.tx.as_mut())
            .await
            .context("mark job failed")?;
            Failed::Final
        } else {
            // Simple increasing delay algorithm: each retry waits an additional second.
            let delay = Duration::from_secs(self.retries as u64 + 1);
            let schedule = OffsetDateTime::now_utc() + delay;
            sqlx::query!(
                "update job set
                    started = null,
                    scheduled_at = $2,
                    failures = array_append(failures, $3),
                    retries = retries + 1
                where id = $1",
                self.id,
                schedule,
                err.into(),
            )
            .execute(self.tx.as_mut())
            .await
            .context("mark job retrying")?;
            Failed::Retry
        };

        self.tx.commit().await.context("commit transaction")?;
        Ok(failed)
    }
}

/// Clean up the task queue. Users should run this periodically.
///
/// Removes tasks that have completed, or that failed more than 1 week ago.
/// The intention with the delay for failed tasks is to facilitate debugging.
#[tracing::instrument]
pub async fn cleanup<'a, E: Executor<'a, Database = Postgres>>(db: E) -> Result<u64> {
    sqlx::query!(
        "delete from job
        where (state = 'Completed' and finished <= now())
        or (state = 'Failed' and finished <= now() - interval '1 week')",
    )
    .execute(db)
    .await
    .context("clean up old jobs")
    .map(|result| result.rows_affected())
}
