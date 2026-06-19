//! Test suite for the Web and headless browsers.

#![cfg(target_arch = "wasm32")]

extern crate wasm_bindgen_test;
use lemons_search::{Search, SearchEngine};
use wasm_bindgen::JsValue;
use wasm_bindgen_test::*;
use web_sys::js_sys;

fn decode_result(entry: &JsValue) -> (usize, Vec<u32>) {
    let index_value =
        js_sys::Reflect::get(entry, &JsValue::from_str("index")).expect("missing index field");
    let index = index_value.as_f64().expect("index should be a number") as usize;

    let ranges_value =
        js_sys::Reflect::get(entry, &JsValue::from_str("r")).expect("missing r field");
    let ranges = js_sys::Uint32Array::new(&ranges_value).to_vec();

    (index, ranges)
}

fn decode_store_result(entry: &JsValue) -> (String, u32, Vec<u32>) {
    let id_value = js_sys::Reflect::get(entry, &JsValue::from_str("id")).expect("missing id field");
    let id = id_value.as_string().expect("id should be a string");

    let score_value =
        js_sys::Reflect::get(entry, &JsValue::from_str("score")).expect("missing score field");
    let score = score_value.as_f64().expect("score should be a number") as u32;

    let ranges_value =
        js_sys::Reflect::get(entry, &JsValue::from_str("r")).expect("missing r field");
    let ranges = js_sys::Uint32Array::new(&ranges_value).to_vec();

    (id, score, ranges)
}

#[wasm_bindgen_test]
fn search_respects_max_results_and_returns_expected_match_set() {
    let mut search = Search::new();
    search.set_max_results(2);
    search.update_index(vec![
        "src/search/preview/index.md".to_string(),
        "src/search/preview/adapter.ts".to_string(),
        "src/search/basic/index.md".to_string(),
        "docs/other-topic.md".to_string(),
    ]);

    let results = search.search("preview");
    assert_eq!(results.length(), 2, "must cap result count to max_results");

    let mut indices: Vec<usize> = results
        .iter()
        .map(|entry| decode_result(&entry).0)
        .collect();
    indices.sort_unstable();

    // Both preview entries should survive the top-2 cutoff for this query.
    assert_eq!(indices, vec![0, 1]);
}

#[wasm_bindgen_test]
fn narrowing_query_keeps_semantically_consistent_match_set() {
    let mut search = Search::new();
    search.set_max_results(10);
    search.update_index(vec![
        "folder/foo bar.md".to_string(),
        "folder/foo baz.md".to_string(),
        "folder/far boo.md".to_string(),
        "folder/qux.md".to_string(),
    ]);

    let broad = search.search("fo");
    let narrow = search.search("foo");

    let broad_ids: std::collections::HashSet<usize> =
        broad.iter().map(|entry| decode_result(&entry).0).collect();
    let narrow_ids: std::collections::HashSet<usize> =
        narrow.iter().map(|entry| decode_result(&entry).0).collect();

    assert!(narrow_ids.iter().all(|idx| broad_ids.contains(idx)));
    assert!(narrow_ids.contains(&0));
    assert!(narrow_ids.contains(&1));
}

#[wasm_bindgen_test]
fn search_returns_compact_highlight_ranges() {
    let mut search = Search::new();
    search.set_max_results(10);
    search.update_index(vec!["folder/foo bar.md".to_string()]);

    let results = search.search("foo");
    assert_eq!(results.length(), 1);

    let (_, ranges) = decode_result(&results.get(0));
    assert_eq!(
        ranges,
        vec![7, 10],
        "contiguous hit should be encoded as one range pair"
    );
}

#[wasm_bindgen_test]
fn fuzzy_datastore_crud_updates_results() {
    let mut engine = SearchEngine::new();
    let store_id = engine.create_datastore("fuzzy");
    let session_id = engine.create_session(&store_id);

    engine.upsert_record(&store_id, "a", "folder/apple.md");
    engine.upsert_record(&store_id, "b", "folder/banana.md");

    let results = engine.search_session(&session_id, "apple");
    assert_eq!(results.length(), 1);
    assert_eq!(decode_store_result(&results.get(0)).0, "a");

    engine.upsert_record(&store_id, "a", "folder/apricot.md");
    let results = engine.search_session(&session_id, "apple");
    assert_eq!(results.length(), 0);

    engine.delete_records(&store_id, vec!["b".to_string()]);
    let results = engine.search_session(&session_id, "banana");
    assert_eq!(results.length(), 0);
}

#[wasm_bindgen_test]
fn fuzzy_sessions_keep_incremental_state_isolated() {
    let mut engine = SearchEngine::new();
    engine.set_max_results(10);
    let store_id = engine.create_datastore("fuzzy");
    engine.upsert_record(&store_id, "a", "folder/foo bar.md");
    engine.upsert_record(&store_id, "b", "folder/foo baz.md");
    engine.upsert_record(&store_id, "c", "folder/far boo.md");

    let session_a = engine.create_session(&store_id);
    let session_b = engine.create_session(&store_id);

    let broad = engine.search_session(&session_a, "fo");
    let narrow = engine.search_session(&session_a, "foo");
    let independent = engine.search_session(&session_b, "boo");

    assert!(broad.length() >= narrow.length());
    assert!(independent.length() > 0);
}

#[wasm_bindgen_test]
fn full_text_datastore_matches_and_tokens_and_highlights_all_occurrences() {
    let mut engine = SearchEngine::new();
    engine.set_max_results(10);
    let store_id = engine.create_datastore("fullText");
    let session_id = engine.create_session(&store_id);

    engine.upsert_record(&store_id, "a", "Apple pie with apple slices");
    engine.upsert_record(&store_id, "b", "Apple tart");
    engine.upsert_record(&store_id, "c", "Banana pie");

    let results = engine.search_session(&session_id, "apple pie");
    assert_eq!(results.length(), 1);

    let (id, score, ranges) = decode_store_result(&results.get(0));
    assert_eq!(id, "a");
    assert!(score > 0);
    assert_eq!(ranges, vec![0, 5, 6, 9, 15, 20]);
}
