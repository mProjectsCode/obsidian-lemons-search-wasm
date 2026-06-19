use std::collections::{BinaryHeap, HashMap};

use nucleo_matcher::{
    pattern::{CaseMatching, Normalization, Pattern},
    Config, Matcher,
};

use crate::{
    datastore::DatastoreKind,
    utils::{NumberedString, ScoredIndex, StoreHealth, StoreSearchResult},
};

#[derive(Default)]
pub(crate) struct FuzzySessionState {
    pattern: Option<Pattern>,
    last_query: String,
    last_match_ids: Vec<usize>,
}

impl FuzzySessionState {
    pub(crate) fn reset(&mut self) {
        self.last_query.clear();
        self.last_match_ids.clear();
    }
}

struct FuzzyRecord {
    id: String,
    text: NumberedString,
}

pub(crate) struct FuzzyDatastore {
    records: Vec<Option<FuzzyRecord>>,
    id_to_index: HashMap<String, usize>,
}

impl FuzzyDatastore {
    pub(crate) fn new() -> Self {
        FuzzyDatastore {
            records: Vec::new(),
            id_to_index: HashMap::new(),
        }
    }

    pub(crate) fn upsert(&mut self, id: String, text: String) {
        if let Some(&idx) = self.id_to_index.get(&id) {
            self.records[idx] = Some(FuzzyRecord {
                id,
                text: NumberedString::new(idx, text),
            });
            return;
        }

        let idx = self.records.len();
        self.id_to_index.insert(id.clone(), idx);
        self.records.push(Some(FuzzyRecord {
            id,
            text: NumberedString::new(idx, text),
        }));
    }

    pub(crate) fn delete(&mut self, id: &str) {
        if let Some(idx) = self.id_to_index.remove(id) {
            self.records[idx] = None;
        }
    }

    pub(crate) fn clear(&mut self) {
        self.records.clear();
        self.id_to_index.clear();
    }

    pub(crate) fn health(&self) -> StoreHealth {
        let live_records = self
            .records
            .iter()
            .filter(|record| record.is_some())
            .count();
        StoreHealth::new(
            DatastoreKind::Fuzzy.as_str(),
            live_records,
            self.records.len().saturating_sub(live_records),
            self.id_to_index.keys().cloned().collect(),
            0,
            0,
        )
    }

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
                .flatten()
                .take(max_results)
                .map(|record| StoreSearchResult::new(record.id.clone(), 0))
                .collect();
        }

        let can_narrow = !session.last_query.is_empty() && query.starts_with(&session.last_query);
        let candidate_indices = if can_narrow && !session.last_match_ids.is_empty() {
            std::mem::take(&mut session.last_match_ids)
        } else {
            (0..self.records.len()).collect()
        };

        let mut matcher = Matcher::new(Config::DEFAULT.match_paths());
        let mut scored = BinaryHeap::<ScoredIndex>::with_capacity(max_results);
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
                let record = self.records.get(scored_idx.idx())?.as_ref()?;
                indices.clear();
                let _ = pattern.indices(record.text.utf32str(), &mut matcher, &mut indices);
                indices.sort_unstable();
                indices.dedup();
                Some(StoreSearchResult::new_from_ranges(
                    record.id.clone(),
                    scored_idx.score(),
                    &indices,
                ))
            })
            .collect()
    }

    fn score_record(
        &self,
        data_idx: usize,
        pattern: &Pattern,
        matcher: &mut Matcher,
        max_results: usize,
        scored: &mut BinaryHeap<ScoredIndex>,
        next_match_ids: &mut Vec<usize>,
    ) {
        let Some(record) = self
            .records
            .get(data_idx)
            .and_then(|record| record.as_ref())
        else {
            return;
        };
        let Some(score) = pattern.score(record.text.utf32str(), matcher) else {
            return;
        };

        next_match_ids.push(data_idx);
        ScoredIndex::push_top_score(scored, ScoredIndex::new(score, data_idx), max_results);
    }
}

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
