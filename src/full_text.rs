use std::collections::HashMap;

use crate::{
    datastore::DatastoreKind,
    utils::{StoreHealth, StoreSearchResult},
};

const BM25_K1: f64 = 1.2;
const BM25_B: f64 = 0.75;
const SCORE_SCALE: f64 = 1000.0;

#[derive(Clone)]
struct TokenOccurrence {
    start: u32,
    end: u32,
}

struct FullTextRecord {
    id: String,
    token_count: usize,
    token_counts: HashMap<String, usize>,
}

pub(crate) struct FullTextDatastore {
    records: Vec<Option<FullTextRecord>>,
    id_to_index: HashMap<String, usize>,
    postings: HashMap<String, HashMap<usize, Vec<TokenOccurrence>>>,
    free_indices: Vec<usize>,
    live_record_count: usize,
    total_token_count: usize,
    posting_occurrence_count: usize,
}

impl FullTextDatastore {
    pub(crate) fn new() -> Self {
        FullTextDatastore {
            records: Vec::new(),
            id_to_index: HashMap::new(),
            postings: HashMap::new(),
            free_indices: Vec::new(),
            live_record_count: 0,
            total_token_count: 0,
            posting_occurrence_count: 0,
        }
    }

    pub(crate) fn upsert(&mut self, id: String, text: String) {
        let idx = self.allocate_record_slot(&id);
        let tokens = tokenize(&text);
        let mut token_counts = HashMap::<String, usize>::new();

        for token in &tokens {
            *token_counts.entry(token.term.clone()).or_insert(0) += 1;
            self.postings
                .entry(token.term.clone())
                .or_default()
                .entry(idx)
                .or_default()
                .push(TokenOccurrence {
                    start: token.start,
                    end: token.end,
                });
        }

        self.id_to_index.insert(id.clone(), idx);
        self.live_record_count += 1;
        self.total_token_count += tokens.len();
        self.posting_occurrence_count += tokens.len();
        self.records[idx] = Some(FullTextRecord {
            id,
            token_count: tokens.len(),
            token_counts,
        });
    }

    pub(crate) fn delete(&mut self, id: &str) {
        if let Some(idx) = self.remove_existing(id) {
            self.free_indices.push(idx);
        }
    }

    pub(crate) fn clear(&mut self) {
        self.records.clear();
        self.id_to_index.clear();
        self.postings.clear();
        self.free_indices.clear();
        self.live_record_count = 0;
        self.total_token_count = 0;
        self.posting_occurrence_count = 0;
    }

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

    pub(crate) fn search(&self, query: &str, max_results: usize) -> Vec<StoreSearchResult> {
        let query_terms = parse_full_text_query(query);
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
        let mut results = Vec::<StoreSearchResult>::new();

        for &idx in smallest_posting.keys() {
            if !postings
                .iter()
                .skip(1)
                .all(|term_postings| term_postings.contains_key(&idx))
            {
                continue;
            }

            let Some(record) = self.records.get(idx).and_then(|record| record.as_ref()) else {
                continue;
            };

            results.push(self.score_record(record, idx, &query_terms, live_docs, avg_len));
        }

        results.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.id.cmp(&b.id)));
        results.truncate(max_results);
        results
    }

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

    fn remove_existing(&mut self, id: &str) -> Option<usize> {
        let idx = self.id_to_index.remove(id)?;

        if let Some(record) = self.records.get_mut(idx).and_then(Option::take) {
            self.live_record_count = self.live_record_count.saturating_sub(1);
            self.total_token_count = self.total_token_count.saturating_sub(record.token_count);
            for term in record.token_counts.keys() {
                self.remove_postings_for_record(term, idx);
            }
        }

        Some(idx)
    }

    fn remove_postings_for_record(&mut self, term: &str, idx: usize) {
        let remove_term = if let Some(posting) = self.postings.get_mut(term) {
            if let Some(occurrences) = posting.remove(&idx) {
                self.posting_occurrence_count = self
                    .posting_occurrence_count
                    .saturating_sub(occurrences.len());
            }
            posting.is_empty()
        } else {
            false
        };

        if remove_term {
            self.postings.remove(term);
        }
    }

    fn lookup_query_postings(
        &self,
        query_terms: &[String],
    ) -> Option<Vec<&HashMap<usize, Vec<TokenOccurrence>>>> {
        let mut postings = Vec::with_capacity(query_terms.len());
        for term in query_terms {
            postings.push(self.postings.get(term)?);
        }
        postings.sort_by_key(|term_postings| term_postings.len());
        Some(postings)
    }

    fn score_record(
        &self,
        record: &FullTextRecord,
        record_idx: usize,
        query_terms: &[String],
        live_docs: usize,
        avg_len: f64,
    ) -> StoreSearchResult {
        let mut score = 0.0;
        let mut highlight_indices = Vec::<u32>::new();

        for term in query_terms {
            let Some(term_postings) = self.postings.get(term) else {
                continue;
            };
            let Some(occurrences) = term_postings.get(&record_idx) else {
                continue;
            };

            score += bm25_score(
                occurrences.len(),
                term_postings.len(),
                record.token_count,
                live_docs,
                avg_len,
            );

            for occurrence in occurrences {
                highlight_indices.extend(occurrence.start..occurrence.end);
            }
        }

        highlight_indices.sort_unstable();
        highlight_indices.dedup();
        StoreSearchResult::new_from_ranges(
            record.id.clone(),
            (score * SCORE_SCALE).round() as u32,
            &highlight_indices,
        )
    }
}

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

struct Token {
    term: String,
    start: u32,
    end: u32,
}

fn tokenize(text: &str) -> Vec<Token> {
    let mut tokens = Vec::<Token>::new();
    let mut term = String::new();
    let mut start = 0_u32;
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
            tokens.push(Token {
                term: std::mem::take(&mut term),
                start,
                end: current,
            });
        }
        current += 1;
    }

    if !term.is_empty() {
        tokens.push(Token {
            term,
            start,
            end: current,
        });
    }

    tokens
}

fn parse_full_text_query(query: &str) -> Vec<String> {
    let mut terms = tokenize(query)
        .into_iter()
        .map(|token| token.term)
        .collect::<Vec<_>>();
    terms.sort();
    terms.dedup();
    terms
}
