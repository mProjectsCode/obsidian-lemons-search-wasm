use std::cmp::Ordering;

use crate::core::matcher_text::MatcherText;

use super::types::{RecordIdx, TermId};

/// Stored full-text document metadata.
pub(super) struct FullTextRecord {
    pub(super) token_count: u32,
    /// Unique terms present in this record, used to remove stale postings.
    pub(super) terms: Vec<TermId>,
}

/// A unique indexed term in the full-text word catalog.
pub(super) struct FullTextTerm {
    pub(super) text: String,
    pub(super) matcher_text: MatcherText,
}

/// A single term occurrence summary inside an inverted-index posting list.
pub(super) struct Posting {
    pub(super) record_idx: RecordIdx,
    pub(super) term_frequency: u32,
}

#[derive(Clone, Copy, Eq, PartialEq)]
/// Heap entry for retaining the highest-scoring records.
pub(super) struct ScoredRecord {
    pub(super) record_idx: RecordIdx,
    pub(super) score: u32,
}

/// One catalog term matched by a single query atom.
#[derive(Clone, Copy)]
pub(super) struct ExpandedTerm {
    pub(super) term_id: TermId,
    pub(super) fuzzy_score: u32,
}

pub(super) type ExpandedTermGroup = Vec<ExpandedTerm>;
pub(super) type ResolvedQuery = (Vec<ExpandedTermGroup>, Vec<ExpandedTermGroup>);

impl Ord for ScoredRecord {
    fn cmp(&self, other: &Self) -> Ordering {
        self.score
            .cmp(&other.score)
            .reverse()
            .then_with(|| self.record_idx.cmp(&other.record_idx))
    }
}

impl PartialOrd for ScoredRecord {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
