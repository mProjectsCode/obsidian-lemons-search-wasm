use std::collections::{HashMap, HashSet};

use super::{model::ExpandedTerm, types::RecordIdx, FullTextDatastore};

/// BM25 term-frequency saturation constant.
const BM25_K1: f64 = 1.2;
/// BM25 document-length normalization constant.
const BM25_B: f64 = 0.75;
/// Blend between BM25 and fuzzy scores for fuzzy-expanded full-text terms.
/// 0.0 ranks only by BM25; 1.0 ranks only by fuzzy match quality.
const FULL_TEXT_FUZZY_SCORE_WEIGHT: f64 = 0.7;
/// Multiplier used to expose normalized floating scores as stable integers.
const SCORE_SCALE: f64 = 100.0;

impl FullTextDatastore {
    /// Scores records that contain at least one term matched by a query atom.
    pub(super) fn score_term_group(
        &self,
        terms: &[ExpandedTerm],
        live_docs: usize,
        avg_len: f64,
    ) -> HashMap<RecordIdx, u32> {
        struct CandidateScore {
            record_idx: RecordIdx,
            bm25_score: f64,
            fuzzy_score: f64,
        }

        let mut candidate_scores = Vec::<CandidateScore>::new();
        let mut max_bm25_score = 0.0_f64;
        let mut max_fuzzy_score = 0.0_f64;

        for term in terms {
            let Some(term_postings) = self.postings.get(term.term_id as usize) else {
                continue;
            };

            for posting in term_postings {
                let Some(record) = self.records.get(posting.record_idx as usize) else {
                    continue;
                };

                let bm25_score = bm25_score(
                    posting.term_frequency as usize,
                    term_postings.len(),
                    record.value.token_count as usize,
                    live_docs,
                    avg_len,
                );
                let fuzzy_score = term.fuzzy_score as f64;
                max_bm25_score = max_bm25_score.max(bm25_score);
                max_fuzzy_score = max_fuzzy_score.max(fuzzy_score);
                candidate_scores.push(CandidateScore {
                    record_idx: posting.record_idx,
                    bm25_score,
                    fuzzy_score,
                });
            }
        }

        let mut scores = HashMap::<RecordIdx, u32>::new();

        for candidate in candidate_scores {
            let bm25_score = normalize_score(candidate.bm25_score, max_bm25_score);
            let fuzzy_score = normalize_score(candidate.fuzzy_score, max_fuzzy_score);
            let score = mixed_full_text_score(bm25_score, fuzzy_score);
            let entry = scores.entry(candidate.record_idx).or_insert(0);
            *entry = (*entry).max(score);
        }

        scores
    }

    /// Returns records rejected by any negated query atom.
    pub(super) fn excluded_records(
        &self,
        negative_groups: &[Vec<ExpandedTerm>],
    ) -> HashSet<RecordIdx> {
        let mut excluded = HashSet::<RecordIdx>::new();
        for group in negative_groups {
            for term in group {
                let Some(term_postings) = self.postings.get(term.term_id as usize) else {
                    continue;
                };
                excluded.extend(term_postings.iter().map(|posting| posting.record_idx));
            }
        }
        excluded
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

fn normalize_score(score: f64, max_score: f64) -> f64 {
    if max_score <= 0.0 {
        return 0.0;
    }

    (score / max_score).clamp(0.0, 1.0)
}

fn mixed_full_text_score(bm25_score: f64, fuzzy_score: f64) -> u32 {
    let fuzzy_weight = FULL_TEXT_FUZZY_SCORE_WEIGHT.clamp(0.0, 1.0);
    let bm25_weight = 1.0 - fuzzy_weight;
    ((bm25_score * bm25_weight + fuzzy_score * fuzzy_weight) * SCORE_SCALE).round() as u32
}
