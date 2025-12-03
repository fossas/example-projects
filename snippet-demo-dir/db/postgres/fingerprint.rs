//! Types and primitives for fingerprints.

use bon::Builder;
use derive_more::{Constructor, From, Into};
use fingerprint::Kind;
use getset::Getters;
use locator::{Locator, StrictLocator};

/// A deterministic identifier for a piece of content.
/// Usually a SHA256 hash of the content after some optional transformation.
#[derive(Clone, Eq, PartialEq, Hash, Constructor, From, Into)]
pub struct FingerprintContent(Vec<u8>);

impl std::fmt::Debug for FingerprintContent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("FingerprintContent")
            .field(&hex::encode(&self.0))
            .finish()
    }
}

impl AsRef<[u8]> for FingerprintContent {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

/// Specifies the method used to generate a fingerprint.
/// This is user provided.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Constructor, From, Into)]
pub struct FingerprintKind(String);

impl AsRef<str> for FingerprintKind {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Specifies a deterministic identifier for content generated with a particular method.
#[derive(Clone, Eq, PartialEq, Hash, Debug, Constructor, Getters)]
#[getset(get = "pub")]
pub struct Fingerprint {
    /// The method used to generate a fingerprint.
    pub kind: FingerprintKind,

    /// A deterministic identifier for a piece of content.
    pub content: FingerprintContent,
}

impl From<Fingerprint> for (FingerprintKind, FingerprintContent) {
    fn from(value: Fingerprint) -> Self {
        (value.kind, value.content)
    }
}

/// Relationship between a [`Fingerprint`] and a [`StrictLocator`].
#[derive(Clone, Eq, PartialEq, Hash, Debug, Constructor, Builder, Getters)]
#[getset(get = "pub")]
pub struct MatchRelation {
    /// The locator of the relation
    pub locator: StrictLocator,

    /// The fingerprint of the location
    pub fingerprint: Fingerprint,
}

impl From<MatchRelation> for (StrictLocator, Fingerprint) {
    fn from(value: MatchRelation) -> Self {
        (value.locator, value.fingerprint)
    }
}

impl From<MatchRelation> for StrictLocator {
    fn from(value: MatchRelation) -> Self {
        value.locator
    }
}

impl From<MatchRelation> for Locator {
    fn from(value: MatchRelation) -> Self {
        value.locator.into_locator()
    }
}

impl AsRef<MatchRelation> for MatchRelation {
    fn as_ref(&self) -> &MatchRelation {
        self
    }
}

/// Selector for [`Relation`]s.
#[derive(Clone, Eq, PartialEq, Hash, Debug, From)]
pub enum MatchSelector {
    /// Selects a specific relation between [`StrictLocator`] and [`Fingerprint`].
    Relation(MatchRelation),

    /// Selects all relations that reference the provided [`StrictLocator`].
    Locator(StrictLocator),

    /// Selects all relations that reference the provided [`Fingerprint`].
    Fingerprint(Fingerprint),
}

impl MatchSelector {
    /// Returns a [`Fingerprint`] if the [`MatchSelector`] is a fingerprint
    /// of the given [`Kind`].
    pub fn as_fingerprint(&self, kind: Kind) -> Option<&Fingerprint> {
        match self {
            MatchSelector::Fingerprint(fp) if fp.kind.as_ref() == kind.as_ref() => Some(fp),
            _ => None,
        }
    }
}

impl AsRef<MatchSelector> for MatchSelector {
    fn as_ref(&self) -> &MatchSelector {
        self
    }
}

/// To query on selectors, they must be flattened
/// and converted to primitive types for the database to use.
///
/// This struct provides _some_ sanity to this operation.
/// Otherwise we just work with tuples which is much worse.
///
/// Unfortunately, there's a fair amount of data duplication;
/// we could be fancy here with like, tracking relations and
/// using getters that dynamically build the vecs
/// but I don't think it's worth it for now unless we find
/// that this turns into an issue.
///
/// Logical groupings are stored below separated by newlines;
/// the logical groupings must have equal counts of elements
/// (since they're used in an `unnest` query)
/// but different logical groupings may have different counts.
/// Again, we could get fancy with compile time validation,
/// but I'm leaving it here for now.
#[derive(Debug, Default, Getters)]
#[getset(get = "pub")]
pub struct FlatSelectors {
    /// Vector of fingerprint kinds.
    fp_kind: Vec<String>,

    /// Vector of fingerprint content.
    fp_content: Vec<Vec<u8>>,

    /// Vector of locators.
    locator: Vec<String>,

    /// Relation kind.
    rel_kind: Vec<String>,

    /// Relaton content.
    rel_content: Vec<Vec<u8>>,

    /// Relation locator.
    rel_locator: Vec<String>,
}

impl FlatSelectors {
    /// Flatten selectors
    pub fn flatten(selectors: &[impl AsRef<MatchSelector>]) -> Self {
        selectors
            .iter()
            .fold(Self::default(), |mut flat, select| match select.as_ref() {
                MatchSelector::Fingerprint(inner) => {
                    flat.push_fp(inner);
                    flat
                }
                MatchSelector::Locator(inner) => {
                    flat.push_loc(inner);
                    flat
                }
                MatchSelector::Relation(inner) => {
                    flat.push_rel(inner);
                    flat
                }
            })
    }

    /// Push fingerprint to vector
    pub fn push_fp(&mut self, fp: &Fingerprint) {
        self.fp_kind.push(fp.kind.as_ref().to_owned());
        self.fp_content.push(fp.content.as_ref().to_owned());
    }

    /// Push locator to vector
    pub fn push_loc(&mut self, loc: &StrictLocator) {
        self.locator.push(loc.to_string());
    }

    /// Push [`MatchRelation`] to vector
    pub fn push_rel(&mut self, rel: &MatchRelation) {
        let (fp, loc) = (&rel.fingerprint, &rel.locator);
        self.rel_kind.push(fp.kind.as_ref().to_owned());
        self.rel_content.push(fp.content.as_ref().to_owned());
        self.rel_locator.push(loc.to_string());
    }
}

impl From<Vec<MatchRelation>> for FlatSelectors {
    fn from(relations: Vec<MatchRelation>) -> Self {
        relations
            .into_iter()
            .fold(Self::default(), |mut flat, rel| {
                let (fp, loc) = (rel.fingerprint, rel.locator);
                flat.rel_kind.push(fp.kind.into());
                flat.rel_content.push(fp.content.into());
                flat.rel_locator.push(loc.to_string());
                flat
            })
    }
}

impl From<Vec<StrictLocator>> for FlatSelectors {
    fn from(locators: Vec<StrictLocator>) -> Self {
        locators.into_iter().fold(Self::default(), |mut flat, loc| {
            flat.locator.push(loc.to_string());
            flat
        })
    }
}

impl From<Vec<Fingerprint>> for FlatSelectors {
    fn from(fingerprints: Vec<Fingerprint>) -> Self {
        fingerprints
            .into_iter()
            .fold(Self::default(), |mut flat, fp| {
                flat.fp_kind.push(fp.kind.into());
                flat.fp_content.push(fp.content.into());
                flat
            })
    }
}
