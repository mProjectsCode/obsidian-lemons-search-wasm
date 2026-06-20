use std::collections::BinaryHeap;

use nucleo_matcher::{Utf32Str, Utf32String};
use web_sys::js_sys;

/// Searchable string stored in the representation expected by `nucleo_matcher`.
pub struct NumberedString {
    utf32: Utf32String,
}

impl NumberedString {
    /// Converts a Rust string into the UTF-32 representation expected by
    /// `nucleo_matcher`.
    pub fn new(string: String) -> Self {
        NumberedString {
            utf32: Utf32String::from(string),
        }
    }

    #[inline]
    /// Borrows the matcher-ready UTF-32 string.
    pub fn utf32str(&self) -> Utf32Str<'_> {
        self.utf32.slice(..)
    }
}

#[derive(Debug, Clone, Copy)]
/// Score/index pair with reversed ordering for bounded min-heap behavior.
pub struct ScoredIndex(u32, usize);

impl ScoredIndex {
    /// Creates a scored index entry.
    pub fn new(score: u32, index: usize) -> Self {
        ScoredIndex(score, index)
    }

    #[inline]
    /// Returns the match score.
    pub fn score(&self) -> u32 {
        self.0
    }

    #[inline]
    /// Returns the associated data index.
    pub fn idx(&self) -> usize {
        self.1
    }

    /// Maintains a heap containing only the highest scoring entries.
    pub fn push_top_score(heap: &mut BinaryHeap<Self>, scored_idx: Self, max_results: usize) {
        if heap.len() < max_results {
            heap.push(scored_idx);
            return;
        }

        let Some(min_idx) = heap.peek() else {
            return;
        };

        if scored_idx.score() > min_idx.score() {
            heap.pop();
            heap.push(scored_idx);
        }
    }
}

impl PartialEq for ScoredIndex {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for ScoredIndex {}

impl PartialOrd for ScoredIndex {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ScoredIndex {
    #[inline]
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.cmp(&other.0).reverse()
    }
}

/// Converts sorted character indices into `[start, end)` range pairs.
fn compact_highlight_ranges(indices: &[u32]) -> Vec<u32> {
    if indices.is_empty() {
        return Vec::new();
    }

    // Worst-case is all isolated indices, which produces 2 entries per index.
    let mut ranges = Vec::with_capacity(indices.len() * 2);

    let mut start = indices[0];
    let mut prev = indices[0];

    for &idx in indices.iter().skip(1) {
        if idx == prev + 1 {
            prev = idx;
            continue;
        }

        ranges.push(start);
        ranges.push(prev + 1);
        start = idx;
        prev = idx;
    }

    ranges.push(start);
    ranges.push(prev + 1);

    ranges
}

#[derive(Debug, Clone, PartialEq)]
/// Datastore search result addressed by external record id.
pub struct StoreSearchResult {
    pub id: String,
    pub score: u32,
    pub highlight_ranges: Vec<u32>,
}

impl StoreSearchResult {
    /// Builds a result without highlight ranges.
    pub fn new(id: String, score: u32) -> Self {
        StoreSearchResult {
            id,
            score,
            highlight_ranges: Vec::new(),
        }
    }

    /// Builds a result and compacts individual matched indices into ranges.
    pub fn new_from_ranges(id: String, score: u32, highlight_ranges: &[u32]) -> Self {
        StoreSearchResult {
            id,
            score,
            highlight_ranges: compact_highlight_ranges(highlight_ranges),
        }
    }

    /// Converts this result into the compact JavaScript object shape.
    pub fn into_js_object(self) -> js_sys::Object {
        let obj = js_sys::Object::new();
        let ranges = js_sys::Uint32Array::from(self.highlight_ranges.as_slice());
        let _ = js_sys::Reflect::set(&obj, &"id".into(), &self.id.into());
        let _ = js_sys::Reflect::set(&obj, &"score".into(), &self.score.into());
        let _ = js_sys::Reflect::set(&obj, &"r".into(), &ranges.into());

        obj
    }
}

#[derive(Debug, Clone, PartialEq)]
/// Diagnostic snapshot for a datastore.
pub struct StoreHealth {
    pub exists: bool,
    pub kind: String,
    pub live_records: usize,
    pub tombstones: usize,
    pub record_ids: Vec<String>,
    pub posting_terms: usize,
    pub posting_occurrences: usize,
}

impl StoreHealth {
    /// Builds a health snapshot for an existing datastore.
    pub fn new(
        kind: &str,
        live_records: usize,
        tombstones: usize,
        mut record_ids: Vec<String>,
        posting_terms: usize,
        posting_occurrences: usize,
    ) -> Self {
        record_ids.sort();
        StoreHealth {
            exists: true,
            kind: kind.to_string(),
            live_records,
            tombstones,
            record_ids,
            posting_terms,
            posting_occurrences,
        }
    }

    /// Builds the health response for a missing datastore id.
    pub fn missing() -> Self {
        StoreHealth {
            exists: false,
            kind: String::new(),
            live_records: 0,
            tombstones: 0,
            record_ids: Vec::new(),
            posting_terms: 0,
            posting_occurrences: 0,
        }
    }

    /// Converts this snapshot into the JavaScript object shape expected by the
    /// TypeScript side.
    pub fn into_js_object(self) -> js_sys::Object {
        let obj = js_sys::Object::new();
        let ids = js_sys::Array::new();
        for id in self.record_ids {
            ids.push(&id.into());
        }
        let _ = js_sys::Reflect::set(&obj, &"exists".into(), &self.exists.into());
        let _ = js_sys::Reflect::set(&obj, &"kind".into(), &self.kind.into());
        let _ = js_sys::Reflect::set(&obj, &"liveRecords".into(), &self.live_records.into());
        let _ = js_sys::Reflect::set(&obj, &"tombstones".into(), &self.tombstones.into());
        let _ = js_sys::Reflect::set(&obj, &"recordIds".into(), &ids.into());
        let _ = js_sys::Reflect::set(&obj, &"postingTerms".into(), &self.posting_terms.into());
        let _ = js_sys::Reflect::set(
            &obj,
            &"postingOccurrences".into(),
            &self.posting_occurrences.into(),
        );

        obj
    }
}

#[cfg(test)]
mod tests {
    use super::compact_highlight_ranges;
    use wasm_bindgen_test::wasm_bindgen_test;

    #[wasm_bindgen_test]
    fn compacts_empty_indices() {
        assert_eq!(compact_highlight_ranges(&[]), Vec::<u32>::new());
    }

    #[wasm_bindgen_test]
    fn compacts_single_contiguous_run() {
        assert_eq!(compact_highlight_ranges(&[3, 4, 5, 6]), vec![3, 7]);
    }

    #[wasm_bindgen_test]
    fn compacts_multiple_runs() {
        assert_eq!(
            compact_highlight_ranges(&[1, 2, 5, 6, 7, 11]),
            vec![1, 3, 5, 8, 11, 12]
        );
    }

    #[wasm_bindgen_test]
    fn preserves_isolated_indices_as_single_length_ranges() {
        assert_eq!(
            compact_highlight_ranges(&[2, 5, 9]),
            vec![2, 3, 5, 6, 9, 10]
        );
    }
}
