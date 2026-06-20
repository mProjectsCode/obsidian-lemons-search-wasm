use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

use crate::{
    datastore::DatastoreKind,
    utils::{StoreHealth, StoreSearchResult},
};

type RecordIdx = u32;
type TermId = u32;

/// BM25 term-frequency saturation constant.
const BM25_K1: f64 = 1.2;
/// BM25 document-length normalization constant.
const BM25_B: f64 = 0.75;
/// Multiplier used to expose floating BM25 scores as stable integer scores.
const SCORE_SCALE: f64 = 1000.0;

/// Stored full-text document metadata.
struct FullTextRecord {
    id: String,
    token_count: usize,
    /// Unique terms present in this record, used to remove stale postings.
    terms: Vec<TermId>,
}

/// A single term occurrence summary inside an inverted-index posting list.
struct Posting {
    record_idx: RecordIdx,
    term_frequency: u32,
}

#[derive(Clone, Copy, Eq, PartialEq)]
/// Heap entry for retaining the highest-scoring records.
struct ScoredRecord {
    record_idx: RecordIdx,
    score: u32,
}

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

/// BM25 full-text datastore with an inverted index.
///
/// Records are stored sparsely so deleted ids can leave reusable slots while
/// postings remain sorted by record index for binary-search intersections.
pub(crate) struct FullTextDatastore {
    records: Vec<Option<FullTextRecord>>,
    id_to_index: HashMap<String, usize>,
    term_to_id: HashMap<String, TermId>,
    /// Posting lists indexed by `TermId`; each list is sorted by `record_idx`.
    postings: Vec<Vec<Posting>>,
    /// Vacant record slots that can be reused on future inserts.
    free_indices: Vec<usize>,
    live_record_count: usize,
    total_token_count: usize,
    /// Total token occurrences across live records, used as a quick index-size
    /// health metric.
    posting_occurrence_count: usize,
}

impl FullTextDatastore {
    /// Creates an empty full-text datastore.
    pub(crate) fn new() -> Self {
        FullTextDatastore {
            records: Vec::new(),
            id_to_index: HashMap::new(),
            term_to_id: HashMap::new(),
            postings: Vec::new(),
            free_indices: Vec::new(),
            live_record_count: 0,
            total_token_count: 0,
            posting_occurrence_count: 0,
        }
    }

    /// Inserts or replaces a record and updates the inverted index.
    pub(crate) fn upsert(&mut self, id: String, text: String) {
        let idx = self.allocate_record_slot(&id);
        let record_idx = as_record_idx(idx);
        // Accumulate per-record frequencies first so each posting list receives
        // one entry per term, not one entry per token occurrence.
        let mut record_postings = HashMap::<TermId, u32>::new();
        let mut token_count = 0;

        tokenize_each(&text, |term, _, _| {
            token_count += 1;
            let term_id = self.intern_term(term);
            *record_postings.entry(term_id).or_default() += 1;
        });

        // Store only unique terms so replacement/deletion can remove stale
        // entries from the affected posting lists.
        let terms = record_postings.keys().copied().collect::<Vec<_>>();

        for (term, term_frequency) in record_postings {
            let postings = self.postings_for_term_mut(term);
            insert_posting(postings, record_idx, term_frequency);
        }

        self.id_to_index.insert(id.clone(), idx);
        self.live_record_count += 1;
        self.total_token_count += token_count;
        self.posting_occurrence_count += token_count;
        self.records[idx] = Some(FullTextRecord {
            id,
            token_count,
            terms,
        });
    }

    /// Deletes a record and makes its slot available for reuse.
    pub(crate) fn delete(&mut self, id: &str) {
        if let Some(idx) = self.remove_existing(id) {
            self.free_indices.push(idx);
        }
    }

    /// Removes all records, terms, postings, and reusable slots.
    pub(crate) fn clear(&mut self) {
        self.records.clear();
        self.id_to_index.clear();
        self.term_to_id.clear();
        self.postings.clear();
        self.free_indices.clear();
        self.live_record_count = 0;
        self.total_token_count = 0;
        self.posting_occurrence_count = 0;
    }

    /// Builds a lightweight health snapshot used by tests and diagnostics.
    pub(crate) fn health(&self) -> StoreHealth {
        StoreHealth::new(
            DatastoreKind::FullText.as_str(),
            self.live_record_count,
            self.records.len().saturating_sub(self.live_record_count),
            self.id_to_index.keys().cloned().collect(),
            self.postings.len(),
            self.posting_occurrence_count,
        )
    }

    /// Searches for documents containing every query term and ranks them using
    /// BM25.
    pub(crate) fn search(&self, query: &str, max_results: usize) -> Vec<StoreSearchResult> {
        let query_terms = self.parse_query_terms(query);
        if query_terms.is_empty() {
            return Vec::new();
        }

        let Some(postings) = self.lookup_query_postings(&query_terms) else {
            return Vec::new();
        };
        let Some(smallest_posting) = postings.first() else {
            return Vec::new();
        };

        let live_docs = self.live_record_count.max(1);
        let avg_len = self.total_token_count as f64 / live_docs as f64;
        let mut scored_records = BinaryHeap::<ScoredRecord>::new();

        // Start with the rarest term's posting list; any matching record must
        // also be present in every other query term's posting list.
        for candidate in *smallest_posting {
            let idx = candidate.record_idx;
            if !postings
                .iter()
                .skip(1)
                .all(|term_postings| find_posting(term_postings, idx).is_some())
            {
                continue;
            }

            let Some(record) = self
                .records
                .get(idx as usize)
                .and_then(|record| record.as_ref())
            else {
                continue;
            };

            push_top_scored_record(
                &mut scored_records,
                ScoredRecord {
                    record_idx: idx,
                    score: self.score_record(record, idx, &postings, live_docs, avg_len),
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
            .filter_map(|scored| self.build_search_result(scored, &postings))
            .collect()
    }

    /// Chooses a record slot, removing an existing record first when the id is
    /// being replaced.
    fn allocate_record_slot(&mut self, id: &str) -> usize {
        if let Some(idx) = self.remove_existing(id) {
            idx
        } else if let Some(idx) = self.free_indices.pop() {
            idx
        } else {
            let idx = self.records.len();
            self.records.push(None);
            idx
        }
    }

    /// Removes a live record and all of its postings, returning its old slot.
    fn remove_existing(&mut self, id: &str) -> Option<usize> {
        let idx = self.id_to_index.remove(id)?;

        if let Some(record) = self.records.get_mut(idx).and_then(Option::take) {
            self.live_record_count = self.live_record_count.saturating_sub(1);
            self.total_token_count = self.total_token_count.saturating_sub(record.token_count);
            self.posting_occurrence_count = self
                .posting_occurrence_count
                .saturating_sub(record.token_count);
            let record_idx = as_record_idx(idx);
            for term in &record.terms {
                self.remove_postings_for_record(*term, record_idx);
            }
        }

        Some(idx)
    }

    /// Removes a record from one sorted posting list.
    fn remove_postings_for_record(&mut self, term: TermId, idx: RecordIdx) {
        if let Some(posting) = self.postings.get_mut(term as usize) {
            if let Ok(posting_idx) = posting.binary_search_by_key(&idx, |entry| entry.record_idx) {
                posting.remove(posting_idx);
            }
        }
    }

    /// Resolves query terms to posting lists ordered from rarest to most common.
    fn lookup_query_postings(&self, query_terms: &[TermId]) -> Option<Vec<&[Posting]>> {
        let mut postings = Vec::with_capacity(query_terms.len());
        for term in query_terms {
            let term_postings = self.postings.get(*term as usize)?.as_slice();
            if term_postings.is_empty() {
                return None;
            }
            postings.push(term_postings);
        }
        postings.sort_by_key(|term_postings| term_postings.len());
        Some(postings)
    }

    /// Returns a stable numeric id for a term, creating a posting list on first
    /// sighting.
    fn intern_term(&mut self, term: String) -> TermId {
        if let Some(term_id) = self.term_to_id.get(&term) {
            return *term_id;
        }

        let term_id = as_term_id(self.postings.len());
        self.term_to_id.insert(term, term_id);
        self.postings.push(Vec::new());
        term_id
    }

    /// Returns the mutable posting list for an already interned term.
    fn postings_for_term_mut(&mut self, term: TermId) -> &mut Vec<Posting> {
        &mut self.postings[term as usize]
    }

    /// Parses, resolves, sorts, and deduplicates query terms.
    fn parse_query_terms(&self, query: &str) -> Vec<TermId> {
        let mut terms = tokenize_terms(query)
            .into_iter()
            .filter_map(|term| self.term_to_id.get(&term).copied())
            .collect::<Vec<_>>();
        terms.sort_unstable();
        terms.dedup();
        terms
    }

    /// Computes the integer BM25 score for one candidate record.
    fn score_record(
        &self,
        record: &FullTextRecord,
        record_idx: RecordIdx,
        postings: &[&[Posting]],
        live_docs: usize,
        avg_len: f64,
    ) -> u32 {
        let mut score = 0.0;

        for term_postings in postings {
            let Some(posting) = find_posting(term_postings, record_idx) else {
                continue;
            };

            score += bm25_score(
                posting.term_frequency as usize,
                term_postings.len(),
                record.token_count,
                live_docs,
                avg_len,
            );
        }

        (score * SCORE_SCALE).round() as u32
    }

    /// Converts a scored record into the JavaScript-facing result shape.
    fn build_search_result(
        &self,
        scored: ScoredRecord,
        _postings: &[&[Posting]],
    ) -> Option<StoreSearchResult> {
        let record = self
            .records
            .get(scored.record_idx as usize)
            .and_then(|record| record.as_ref())?;
        Some(StoreSearchResult::new(record.id.clone(), scored.score))
    }

    /// Returns a record id for deterministic tie-breaking during sorting.
    fn record_id(&self, record_idx: RecordIdx) -> &str {
        self.records
            .get(record_idx as usize)
            .and_then(|record| record.as_ref())
            .map_or("", |record| record.id.as_str())
    }
}

/// Computes the BM25 contribution of one matched term.
fn bm25_score(
    term_frequency: usize,
    document_frequency: usize,
    document_len: usize,
    live_docs: usize,
    avg_len: f64,
) -> f64 {
    let tf = term_frequency as f64;
    let df = document_frequency as f64;
    let idf = ((live_docs as f64 - df + 0.5) / (df + 0.5) + 1.0).ln();
    let length = document_len.max(1) as f64;
    let denom = tf + BM25_K1 * (1.0 - BM25_B + BM25_B * (length / avg_len.max(1.0)));

    idf * ((tf * (BM25_K1 + 1.0)) / denom)
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

/// Inserts or replaces a posting while preserving record-index sort order.
fn insert_posting(postings: &mut Vec<Posting>, record_idx: RecordIdx, term_frequency: u32) {
    if postings
        .last()
        .is_none_or(|posting| posting.record_idx < record_idx)
    {
        postings.push(Posting {
            record_idx,
            term_frequency,
        });
        return;
    }

    match postings.binary_search_by_key(&record_idx, |posting| posting.record_idx) {
        Ok(idx) => postings[idx].term_frequency = term_frequency,
        Err(idx) => postings.insert(
            idx,
            Posting {
                record_idx,
                term_frequency,
            },
        ),
    }
}

/// Finds a record inside a sorted posting list.
fn find_posting(postings: &[Posting], record_idx: RecordIdx) -> Option<&Posting> {
    postings
        .binary_search_by_key(&record_idx, |posting| posting.record_idx)
        .ok()
        .map(|idx| &postings[idx])
}

/// Tokenizes text into lowercase alphanumeric terms and reports character
/// offsets for callers that need positional data.
fn tokenize_each(text: &str, mut emit: impl FnMut(String, u32, u32)) {
    let mut term = String::new();
    let mut start = 0_u32;
    // Offsets are counted in Unicode scalar values to match `chars()` traversal.
    let mut current = 0_u32;

    for ch in text.chars() {
        if ch.is_alphanumeric() {
            if term.is_empty() {
                start = current;
            }
            for lowered in ch.to_lowercase() {
                term.push(lowered);
            }
        } else if !term.is_empty() {
            emit(std::mem::take(&mut term), start, current);
        }
        current += 1;
    }

    if !term.is_empty() {
        emit(term, start, current);
    }
}

/// Returns only the normalized terms from `tokenize_each`.
fn tokenize_terms(text: &str) -> Vec<String> {
    let mut terms = Vec::<String>::new();
    tokenize_each(text, |term, _, _| terms.push(term));
    terms
}

/// Converts a sparse record vector index into the compact posting-list type.
fn as_record_idx(idx: usize) -> RecordIdx {
    u32::try_from(idx).expect("full-text record count exceeded u32::MAX")
}

/// Converts a posting-list vector index into the compact term id type.
fn as_term_id(idx: usize) -> TermId {
    u32::try_from(idx).expect("full-text term count exceeded u32::MAX")
}

#[cfg(test)]
mod tests {
    use super::FullTextDatastore;

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
    }
}
