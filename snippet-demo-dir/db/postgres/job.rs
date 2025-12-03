//! Types and primitives for jobs.

use color_eyre::eyre::{Report, Result, eyre};
use duplicate::duplicate;
use serde::{Deserialize, Serialize};
use strum::{AsRefStr, Display, EnumIter, EnumString, FromRepr};

use crate::db::postgres::{DecodePostgres, EncodePostgres, TryDecodePostgres};

/// Represents the possible states of a job as stored in a PostgreSQL text column.
#[derive(
    Copy,
    Clone,
    Eq,
    PartialEq,
    PartialOrd,
    Ord,
    Hash,
    Debug,
    Default,
    Serialize,
    Deserialize,
    FromRepr,
    EnumIter,
    Display,
    EnumString,
    AsRefStr,
)]
pub enum JobState {
    /// The job is ongoing.
    #[default]
    Pending,

    /// The job has completed.
    Completed,

    /// The job has failed.
    Failed,
}

impl EncodePostgres for JobState {
    type Base = String;

    fn to_db(self) -> Self::Base {
        self.as_ref().to_string()
    }
}

impl TryDecodePostgres for JobState {
    type Base = String;
    type Err = Report;

    fn try_from_db(value: Self::Base) -> Result<Self, Self::Err> {
        value
            .parse::<JobState>()
            .map_err(|_| eyre!("'{}' is not a valid JobState", value))
    }
}

/// The ID of the job created from a request.
#[derive(Copy, Clone, Eq, PartialEq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct JobId(u64);

/// The unique ID of a build.
#[derive(Copy, Clone, Eq, PartialEq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct BuildId(u64);

duplicate! {
    [
        base    dest    name;
        [ i64 ] [ u64 ] [ JobId ];
        [ i64 ] [ u64 ] [ BuildId ];
    ]
    impl EncodePostgres for name {
        type Base = base;
        fn to_db(self) -> Self::Base {
            self.0 as base
        }
    }
    impl DecodePostgres for name {
        type Base = base;
        fn from_db(value: Self::Base) -> Self {
            Self(value as dest)
        }
    }
}
