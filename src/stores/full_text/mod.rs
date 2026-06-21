use std::collections::{BinaryHeap, HashMap};

use crate::{
    core::{
        matcher_text::MatcherText,
        record_table::{RecordTable, StoredRecord},
        search_result::SearchResult,
        store_health::DatastoreHealth,
        text_tokenizer::tokenize_each,
    },
    stores::DatastoreKind,
};

mod model;
mod postings;
mod query;
mod scoring;
mod types;
use model::{ExpandedTerm, FullTextRecord, FullTextTerm, Posting, ScoredRecord};
use postings::insert_posting;
use types::{
    record_index_from_usize, term_id_from_usize, token_count_from_usize, RecordIdx, TermId,
};

/// BM25 full-text datastore with an inverted index.
///
/// Records are stored sparsely so deleted ids can leave reusable slots while
/// postings remain sorted by record index for binary-search intersections.
pub(crate) struct FullTextDatastore {
    records: RecordTable<FullTextRecord>,
    term_to_id: HashMap<String, TermId>,
    terms: Vec<FullTextTerm>,
    /// Posting lists indexed by `TermId`; each list is sorted by `record_idx`.
    postings: Vec<Vec<Posting>>,
    total_token_count: usize,
    /// Total token occurrences across live records, used as a quick index-size
    /// health metric.
    posting_occurrence_count: usize,
    bulk_loading: bool,
    fuzzy_search: bool,
}

impl FullTextDatastore {
    /// Creates an empty full-text datastore.
    pub(crate) fn new() -> Self {
        FullTextDatastore {
            records: RecordTable::new(),
            term_to_id: HashMap::new(),
            terms: Vec::new(),
            postings: Vec::new(),
            total_token_count: 0,
            posting_occurrence_count: 0,
            bulk_loading: false,
            fuzzy_search: true,
        }
    }

    /// Sets whether fuzzy matching is used during search.
    pub(crate) fn set_fuzzy_search(&mut self, enabled: bool) {
        self.fuzzy_search = enabled;
    }

    /// Inserts or replaces a record and updates the inverted index.
    pub(crate) fn upsert(&mut self, id: String, text: String) {
        let (record_postings, token_count) = self.build_record_postings(&text);
        let terms = record_postings
            .iter()
            .map(|(term, _)| *term)
            .collect::<Vec<_>>();
        let (idx, replaced) = self
            .records
            .upsert(id, FullTextRecord { token_count, terms });
        if let Some(record) = replaced {
            self.remove_record_from_index(idx, record);
        }

        let record_idx = record_index_from_usize(idx);
        let bulk_loading = self.bulk_loading;
        for (term, term_frequency) in record_postings {
            let postings = self.postings_for_term_mut(term);
            if bulk_loading {
                postings.push(Posting {
                    record_idx,
                    term_frequency,
                });
            } else {
                insert_posting(postings, record_idx, term_frequency);
            }
        }

        self.total_token_count += token_count as usize;
        self.posting_occurrence_count += token_count as usize;
    }

    /// Deletes a record and makes its slot available for reuse.
    pub(crate) fn delete(&mut self, id: &str) {
        if let Some((idx, record)) = self.records.delete(id) {
            self.remove_record_from_index(idx, record);
        }
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

    /// Removes all records, terms, postings, and reusable slots.
    pub(crate) fn clear(&mut self) {
        self.records.clear();
        self.term_to_id.clear();
        self.terms.clear();
        self.postings.clear();
        self.total_token_count = 0;
        self.posting_occurrence_count = 0;
        self.bulk_loading = false;
        self.fuzzy_search = true;
    }

    /// Starts a bulk rebuild. Records inserted before `finish_bulk_load` use
    /// append-only posting writes and are sorted once at the end.
    pub(crate) fn begin_bulk_load(&mut self) {
        self.clear();
        self.bulk_loading = true;
    }

    /// Finalizes a bulk rebuild by restoring sorted posting-list invariants.
    pub(crate) fn finish_bulk_load(&mut self) {
        if !self.bulk_loading {
            return;
        }

        for postings in &mut self.postings {
            postings.sort_unstable_by_key(|posting| posting.record_idx);
        }
        self.bulk_loading = false;
    }

    /// Builds a lightweight health snapshot used by tests and diagnostics.
    pub(crate) fn health(&self) -> DatastoreHealth {
        DatastoreHealth::new(
            DatastoreKind::FullText.as_str(),
            self.records.live_record_count(),
            self.records.tombstone_count(),
            self.records.record_ids(),
            self.postings.len(),
            self.posting_occurrence_count,
        )
    }

    /// Searches for documents containing every query term and ranks them using
    /// BM25.
    pub(crate) fn search(&self, query: &str, max_results: usize) -> Vec<SearchResult> {
        if self.bulk_loading {
            return Vec::new();
        }

        let Some(query) = self.resolve_query(query) else {
            return Vec::new();
        };

        let (positive_groups, negative_groups) = query;
        if positive_groups.is_empty() {
            return Vec::new();
        }

        let live_docs = self.records.live_record_count().max(1);
        let avg_len = self.total_token_count as f64 / live_docs as f64;
        let mut positive_scores = positive_groups
            .iter()
            .map(|group| self.score_term_group(group, live_docs, avg_len))
            .collect::<Vec<_>>();
        if positive_scores.iter().any(|scores| scores.is_empty()) {
            return Vec::new();
        }
        positive_scores.sort_by_key(|scores| scores.len());

        let excluded_records = self.excluded_records(&negative_groups);
        let mut scored_records = BinaryHeap::<ScoredRecord>::new();
        let Some(smallest_scores) = positive_scores.first() else {
            return Vec::new();
        };

        // Start with the rarest expanded atom's record set; any matching record
        // must also be present in every other positive query atom's record set.
        for (&idx, &base_score) in smallest_scores {
            if excluded_records.contains(&idx)
                || !positive_scores
                    .iter()
                    .skip(1)
                    .all(|scores| scores.contains_key(&idx))
            {
                continue;
            }

            let score = base_score
                + positive_scores
                    .iter()
                    .skip(1)
                    .filter_map(|scores| scores.get(&idx))
                    .sum::<u32>();

            push_top_scored_record(
                &mut scored_records,
                ScoredRecord {
                    record_idx: idx,
                    score,
                },
                max_results,
            );
        }

        let mut scored_records = scored_records.into_vec();
        scored_records.sort_by(|a, b| {
            b.score.cmp(&a.score).then_with(|| {
                self.record_id(a.record_idx)
                    .cmp(self.record_id(b.record_idx))
            })
        });

        scored_records
            .into_iter()
            .filter_map(|scored| self.build_search_result(scored, &positive_groups))
            .collect()
    }

    /// Removes a live record from all index side structures.
    fn remove_record_from_index(&mut self, idx: usize, record: StoredRecord<FullTextRecord>) {
        self.total_token_count = self
            .total_token_count
            .saturating_sub(record.value.token_count as usize);
        self.posting_occurrence_count = self
            .posting_occurrence_count
            .saturating_sub(record.value.token_count as usize);
        let record_idx = record_index_from_usize(idx);
        for term in &record.value.terms {
            self.remove_postings_for_record(*term, record_idx);
        }
    }

    /// Removes a record from one sorted posting list.
    fn remove_postings_for_record(&mut self, term: TermId, idx: RecordIdx) {
        if let Some(posting) = self.postings.get_mut(term as usize) {
            let posting_idx = if self.bulk_loading {
                posting.iter().position(|entry| entry.record_idx == idx)
            } else {
                posting
                    .binary_search_by_key(&idx, |entry| entry.record_idx)
                    .ok()
            };

            if let Some(posting_idx) = posting_idx {
                posting.remove(posting_idx);
            }
        }
    }

    /// Returns a stable numeric id for a term, creating a posting list on first
    /// sighting.
    fn intern_term(&mut self, term: String) -> TermId {
        if let Some(term_id) = self.term_to_id.get(&term) {
            return *term_id;
        }

        let term_id = term_id_from_usize(self.postings.len());
        self.terms.push(FullTextTerm {
            text: term.clone(),
            matcher_text: MatcherText::new(term.clone()),
        });
        self.term_to_id.insert(term, term_id);
        self.postings.push(Vec::new());
        term_id
    }

    /// Returns the mutable posting list for an already interned term.
    fn postings_for_term_mut(&mut self, term: TermId) -> &mut Vec<Posting> {
        &mut self.postings[term as usize]
    }

    /// Builds one compact posting entry per unique term in a record.
    fn build_record_postings(&mut self, text: &str) -> (Vec<(TermId, u32)>, u32) {
        let mut terms = Vec::<TermId>::new();
        tokenize_each(text, |term, _, _| {
            terms.push(self.intern_term(term));
        });
        terms.sort_unstable();

        let token_count = token_count_from_usize(terms.len());
        let mut postings = Vec::with_capacity(terms.len());
        for term in terms {
            if let Some((last_term, term_frequency)) = postings.last_mut() {
                if *last_term == term {
                    *term_frequency += 1;
                    continue;
                }
            }
            postings.push((term, 1));
        }

        (postings, token_count)
    }

    /// Converts a scored record into the JavaScript-facing result shape.
    fn build_search_result(
        &self,
        scored: ScoredRecord,
        positive_groups: &[Vec<ExpandedTerm>],
    ) -> Option<SearchResult> {
        let record = self.records.get(scored.record_idx as usize)?;
        Some(SearchResult::with_matched_terms(
            record.id.to_string(),
            scored.score,
            self.matched_terms_for_record(&record.value.terms, positive_groups),
        ))
    }

    /// Returns a record id for deterministic tie-breaking during sorting.
    fn record_id(&self, record_idx: RecordIdx) -> &str {
        self.records
            .get(record_idx as usize)
            .map_or("", |record| record.id.as_ref())
    }
}

/// Maintains a bounded heap containing only the top scoring records.
fn push_top_scored_record(
    heap: &mut BinaryHeap<ScoredRecord>,
    scored_record: ScoredRecord,
    max_results: usize,
) {
    if heap.len() < max_results {
        heap.push(scored_record);
        return;
    }

    let Some(worst_record) = heap.peek() else {
        return;
    };

    if scored_record.score > worst_record.score {
        heap.pop();
        heap.push(scored_record);
    }
}

#[cfg(test)]
mod tests {
    use super::{model::ExpandedTerm, query::filter_fuzzy_expanded_terms, FullTextDatastore};

    #[test]
    fn updates_existing_record_and_removes_stale_postings() {
        let mut store = FullTextDatastore::new();
        store.upsert("a".to_string(), "apple pie".to_string());
        store.upsert("a".to_string(), "banana tart".to_string());

        assert!(store.search("apple", 10).is_empty());
        assert_eq!(store.search("banana", 10)[0].id, "a");
    }

    #[test]
    fn deletes_record_from_posting_vectors() {
        let mut store = FullTextDatastore::new();
        store.upsert("a".to_string(), "apple pie".to_string());
        store.upsert("b".to_string(), "apple tart".to_string());
        store.delete("a");

        let results = store.search("apple", 10);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "b");
    }

    #[test]
    fn multi_term_search_preserves_highlights() {
        let mut store = FullTextDatastore::new();
        store.upsert("a".to_string(), "Apple pie with apple slices".to_string());
        store.upsert("b".to_string(), "Apple tart".to_string());

        let results = store.search("apple pie", 10);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "a");
        assert!(results[0].highlight_ranges.is_empty());
        assert_eq!(results[0].matched_terms, vec!["apple", "pie"]);
    }

    #[test]
    fn fuzzy_query_terms_expand_against_word_catalog() {
        let mut store = FullTextDatastore::new();
        store.upsert("a".to_string(), "Apple pie with apple slices".to_string());
        store.upsert("b".to_string(), "Banana tart".to_string());

        let results = store.search("apl sli", 10);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "a");
        assert!(results[0].highlight_ranges.is_empty());
        assert_eq!(results[0].matched_terms, vec!["apple", "slices"]);
    }

    #[test]
    fn fuzzy_query_expansion_caps_positive_terms() {
        let mut store = FullTextDatastore::new();
        for idx in 0..60 {
            store.upsert(format!("id-{idx}"), format!("a{idx:02}"));
        }

        let (positive_groups, _) = store.resolve_query("a").expect("query should resolve");

        assert_eq!(positive_groups.len(), 1);
        assert_eq!(positive_groups[0].len(), 50);
    }

    #[test]
    fn fuzzy_query_expansion_applies_relative_score_floor() {
        let terms = filter_fuzzy_expanded_terms(
            vec![
                ExpandedTerm {
                    term_id: 1,
                    fuzzy_score: 100,
                },
                ExpandedTerm {
                    term_id: 2,
                    fuzzy_score: 70,
                },
                ExpandedTerm {
                    term_id: 3,
                    fuzzy_score: 69,
                },
            ],
            false,
        );

        assert_eq!(
            terms.iter().map(|term| term.term_id).collect::<Vec<_>>(),
            vec![1, 2]
        );
    }

    #[test]
    fn parsed_atom_syntax_applies_to_full_text_term_expansion() {
        let mut store = FullTextDatastore::new();
        store.upsert("a".to_string(), "Apple pie".to_string());
        store.upsert("b".to_string(), "Pineapple tart".to_string());
        store.upsert("c".to_string(), "Crabapple crumble".to_string());

        let prefix_results = store.search("^app", 10);
        let substring_results = store.search("'apple", 10);

        assert_eq!(
            prefix_results
                .iter()
                .map(|result| result.id.as_str())
                .collect::<Vec<_>>(),
            vec!["a"]
        );
        assert_eq!(prefix_results[0].matched_terms, vec!["apple"]);
        assert_eq!(substring_results.len(), 3);
    }

    #[test]
    fn negated_atoms_exclude_records_with_matching_catalog_terms() {
        let mut store = FullTextDatastore::new();
        store.upsert("a".to_string(), "Apple pie".to_string());
        store.upsert("b".to_string(), "Apple tart".to_string());
        store.upsert("c".to_string(), "Banana pie".to_string());

        let results = store.search("apple !tart", 10);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "a");
    }

    #[test]
    fn bulk_load_finalizes_queryable_postings() {
        let mut store = FullTextDatastore::new();
        store.begin_bulk_load();
        store.upsert("a".to_string(), "apple pie".to_string());
        store.upsert("b".to_string(), "banana pie".to_string());
        store.finish_bulk_load();

        let results = store.search("apple pie", 10);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "a");
    }

    #[test]
    fn bulk_load_rejects_searches_before_finalization() {
        let mut store = FullTextDatastore::new();
        store.begin_bulk_load();
        store.upsert("a".to_string(), "apple pie".to_string());
        store.upsert("b".to_string(), "banana pie".to_string());

        let results = store.search("banana pie", 10);

        assert!(results.is_empty());
    }
}
