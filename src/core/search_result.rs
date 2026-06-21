use web_sys::js_sys;

/// Converts sorted character indices into `[start, end)` range pairs.
fn compact_highlight_ranges(indices: &[u32]) -> Vec<u32> {
    if indices.is_empty() {
        return Vec::new();
    }

    // Worst-case is all isolated indices, which produces 2 entries per index.
    let mut ranges = Vec::with_capacity(indices.len() * 2);

    let mut start = indices[0];
    let mut prev = indices[0];

    for &index in indices.iter().skip(1) {
        if index == prev + 1 {
            prev = index;
            continue;
        }

        ranges.push(start);
        ranges.push(prev + 1);
        start = index;
        prev = index;
    }

    ranges.push(start);
    ranges.push(prev + 1);

    ranges
}

/// Datastore search result addressed by external record id.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SearchResult {
    pub(crate) id: String,
    pub(crate) score: u32,
    pub(crate) highlight_ranges: Vec<u32>,
    pub(crate) matched_terms: Vec<String>,
}

impl SearchResult {
    /// Builds a result without highlight ranges.
    pub(crate) fn new(id: String, score: u32) -> Self {
        SearchResult {
            id,
            score,
            highlight_ranges: Vec::new(),
            matched_terms: Vec::new(),
        }
    }

    /// Builds a result and compacts individual matched indices into ranges.
    pub(crate) fn with_highlight_indices(
        id: String,
        score: u32,
        highlight_indices: &[u32],
    ) -> Self {
        SearchResult {
            id,
            score,
            highlight_ranges: compact_highlight_ranges(highlight_indices),
            matched_terms: Vec::new(),
        }
    }

    /// Builds a result with search-authoritative matched terms.
    pub(crate) fn with_matched_terms(id: String, score: u32, matched_terms: Vec<String>) -> Self {
        SearchResult {
            id,
            score,
            highlight_ranges: Vec::new(),
            matched_terms,
        }
    }

    /// Converts this result into the compact JavaScript object shape.
    pub(crate) fn into_js_object(self) -> js_sys::Object {
        let obj = js_sys::Object::new();
        let ranges = js_sys::Uint32Array::from(self.highlight_ranges.as_slice());
        let _ = js_sys::Reflect::set(&obj, &"id".into(), &self.id.into());
        let _ = js_sys::Reflect::set(&obj, &"score".into(), &self.score.into());
        let _ = js_sys::Reflect::set(&obj, &"r".into(), &ranges.into());
        if !self.matched_terms.is_empty() {
            let terms = js_sys::Array::new();
            for term in self.matched_terms {
                terms.push(&term.into());
            }
            let _ = js_sys::Reflect::set(&obj, &"m".into(), &terms.into());
        }

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
