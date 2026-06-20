use std::collections::BinaryHeap;

use nucleo_matcher::{
    pattern::{CaseMatching, Normalization, Pattern},
    Config, Matcher,
};

use crate::{
    datastore::DatastoreKind,
    record_table::RecordTable,
    utils::{NumberedString, ScoredIndex, StoreHealth, StoreSearchResult},
};

#[derive(Default)]
/// Incremental fuzzy-search cache owned by a single search session.
pub(crate) struct FuzzySessionState {
    pattern: Option<Pattern>,
    /// Previous query text used to detect prefix narrowing.
    last_query: String,
    /// Record indices that matched the previous query and can be rescored for
    /// a narrower prefix query.
    last_match_ids: Vec<usize>,
}

impl FuzzySessionState {
    /// Clears incremental state after the datastore changes.
    pub(crate) fn reset(&mut self) {
        self.last_query.clear();
        self.last_match_ids.clear();
    }
}

/// One fuzzy-searchable record.
struct FuzzyRecord {
    text: NumberedString,
}

/// Sparse fuzzy-search datastore backed by `nucleo_matcher`.
///
/// Deletions leave reusable tombstones so existing numeric indices remain
/// stable for cached session candidates without growing the slot vector
/// indefinitely.
pub(crate) struct FuzzyDatastore {
    records: RecordTable<FuzzyRecord>,
}

impl FuzzyDatastore {
    /// Creates an empty fuzzy datastore.
    pub(crate) fn new() -> Self {
        FuzzyDatastore {
            records: RecordTable::new(),
        }
    }

    /// Inserts or replaces a record by its external id.
    pub(crate) fn upsert(&mut self, id: String, text: String) {
        self.records.upsert(
            id,
            FuzzyRecord {
                text: NumberedString::new(text),
            },
        );
    }

    /// Removes a record, leaving a reusable tombstone at its old numeric index.
    pub(crate) fn delete(&mut self, id: &str) {
        self.records.delete(id);
    }

    /// Removes every record whose external id starts with `prefix`.
    pub(crate) fn delete_by_prefix(&mut self, prefix: &str) {
        let ids = self
            .records
            .record_ids()
            .into_iter()
            .filter(|id| id.starts_with(prefix))
            .collect::<Vec<_>>();
        for id in ids {
            self.delete(&id);
        }
    }

    /// Removes every record and tombstone.
    pub(crate) fn clear(&mut self) {
        self.records.clear();
    }

    /// Builds a lightweight health snapshot used by tests and diagnostics.
    pub(crate) fn health(&self) -> StoreHealth {
        StoreHealth::new(
            DatastoreKind::Fuzzy.as_str(),
            self.records.live_record_count(),
            self.records.tombstone_count(),
            self.records.record_ids(),
            0,
            0,
        )
    }

    /// Searches records, reusing prior session matches when the query narrows.
    pub(crate) fn search(
        &self,
        session: &mut FuzzySessionState,
        query: &str,
        max_results: usize,
    ) -> Vec<StoreSearchResult> {
        reparse_pattern(&mut session.pattern, query);

        let Some(pattern) = session.pattern.as_ref() else {
            return Vec::new();
        };

        if pattern.atoms.is_empty() {
            session.last_query.clear();
            session.last_match_ids.clear();
            return self
                .records
                .iter()
                .take(max_results)
                .map(|record| StoreSearchResult::new(record.id.to_string(), 0))
                .collect();
        }

        // A prefix extension can only remove fuzzy matches, so rescoring the
        // prior hit set avoids scanning every record on each keystroke.
        let can_narrow = !session.last_query.is_empty() && query.starts_with(&session.last_query);
        let candidate_indices = if can_narrow && !session.last_match_ids.is_empty() {
            std::mem::take(&mut session.last_match_ids)
        } else {
            (0..self.records.slot_count()).collect()
        };

        let mut matcher = Matcher::new(Config::DEFAULT.match_paths());
        let mut scored = BinaryHeap::<ScoredIndex>::with_capacity(max_results);
        // All matches are retained for future narrowing, while `scored` only
        // keeps the top `max_results` entries needed for this response.
        let mut next_match_ids = Vec::<usize>::with_capacity(candidate_indices.len());

        for data_idx in candidate_indices {
            self.score_record(
                data_idx,
                pattern,
                &mut matcher,
                max_results,
                &mut scored,
                &mut next_match_ids,
            );
        }

        session.last_query.clear();
        session.last_query.push_str(query);
        session.last_match_ids = next_match_ids;

        let mut indices = Vec::<u32>::new();
        scored
            .into_sorted_vec()
            .into_iter()
            .filter_map(|scored_idx| {
                let record = self.records.get(scored_idx.idx())?;
                indices.clear();
                let _ = pattern.indices(record.value.text.utf32str(), &mut matcher, &mut indices);
                indices.sort_unstable();
                indices.dedup();
                Some(StoreSearchResult::new_from_ranges(
                    record.id.to_string(),
                    scored_idx.score(),
                    &indices,
                ))
            })
            .collect()
    }

    /// Scores one candidate and records it for both current results and future
    /// incremental narrowing.
    fn score_record(
        &self,
        data_idx: usize,
        pattern: &Pattern,
        matcher: &mut Matcher,
        max_results: usize,
        scored: &mut BinaryHeap<ScoredIndex>,
        next_match_ids: &mut Vec<usize>,
    ) {
        let Some(record) = self.records.get(data_idx) else {
            return;
        };
        let Some(score) = pattern.score(record.value.text.utf32str(), matcher) else {
            return;
        };

        next_match_ids.push(data_idx);
        ScoredIndex::push_top_score(scored, ScoredIndex::new(score, data_idx), max_results);
    }
}

/// Reuses the existing `nucleo_matcher` pattern allocation when available.
pub(crate) fn reparse_pattern(pattern: &mut Option<Pattern>, query: &str) {
    if let Some(pattern) = pattern {
        pattern.reparse(query, CaseMatching::Smart, Normalization::Smart);
    } else {
        *pattern = Some(Pattern::parse(
            query,
            CaseMatching::Smart,
            Normalization::Smart,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::FuzzyDatastore;

    #[test]
    fn reuses_deleted_slots_and_tracks_health_without_scanning() {
        let mut store = FuzzyDatastore::new();
        store.upsert("a".to_string(), "apple".to_string());
        store.upsert("b".to_string(), "banana".to_string());
        store.delete("a");

        let health = store.health();
        assert_eq!(health.live_records, 1);
        assert_eq!(health.tombstones, 1);

        store.upsert("c".to_string(), "citrus".to_string());

        let health = store.health();
        assert_eq!(health.live_records, 2);
        assert_eq!(health.tombstones, 0);
        assert_eq!(health.record_ids, vec!["b", "c"]);
    }
}
