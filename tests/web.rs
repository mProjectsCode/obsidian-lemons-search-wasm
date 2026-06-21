//! Test suite for the Web and headless browsers.

#![cfg(target_arch = "wasm32")]

extern crate wasm_bindgen_test;
use lemons_search::SearchEngine;
use wasm_bindgen::JsValue;
use wasm_bindgen_test::*;
use web_sys::js_sys;

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

fn decode_matched_terms(entry: &JsValue) -> Vec<String> {
    let Ok(terms_value) = js_sys::Reflect::get(entry, &JsValue::from_str("m")) else {
        return Vec::new();
    };
    if terms_value.is_undefined() {
        return Vec::new();
    }

    js_sys::Array::from(&terms_value)
        .iter()
        .filter_map(|term| term.as_string())
        .collect()
}

#[wasm_bindgen_test]
fn search_respects_max_results_and_returns_expected_match_set() {
    let mut engine = SearchEngine::new();
    engine.set_max_results(2);
    let store_id = engine.create_datastore("fuzzy");
    let session_id = engine.create_session(&store_id);
    engine.upsert_record(&store_id, "0", "src/search/preview/index.md");
    engine.upsert_record(&store_id, "1", "src/search/preview/adapter.ts");
    engine.upsert_record(&store_id, "2", "src/search/basic/index.md");
    engine.upsert_record(&store_id, "3", "docs/other-topic.md");

    let results = engine.search_session(&session_id, "preview");
    assert_eq!(results.length(), 2, "must cap result count to max_results");

    let mut ids: Vec<String> = results
        .iter()
        .map(|entry| decode_store_result(&entry).0)
        .collect();
    ids.sort_unstable();

    // Both preview entries should survive the top-2 cutoff for this query.
    assert_eq!(ids, vec!["0", "1"]);
}

#[wasm_bindgen_test]
fn narrowing_query_keeps_semantically_consistent_match_set() {
    let mut engine = SearchEngine::new();
    engine.set_max_results(10);
    let store_id = engine.create_datastore("fuzzy");
    let session_id = engine.create_session(&store_id);
    engine.upsert_record(&store_id, "0", "folder/foo bar.md");
    engine.upsert_record(&store_id, "1", "folder/foo baz.md");
    engine.upsert_record(&store_id, "2", "folder/far boo.md");
    engine.upsert_record(&store_id, "3", "folder/qux.md");

    let broad = engine.search_session(&session_id, "fo");
    let narrow = engine.search_session(&session_id, "foo");

    let broad_ids: std::collections::HashSet<String> = broad
        .iter()
        .map(|entry| decode_store_result(&entry).0)
        .collect();
    let narrow_ids: std::collections::HashSet<String> = narrow
        .iter()
        .map(|entry| decode_store_result(&entry).0)
        .collect();

    assert!(narrow_ids.iter().all(|id| broad_ids.contains(id)));
    assert!(narrow_ids.contains("0"));
    assert!(narrow_ids.contains("1"));
}

#[wasm_bindgen_test]
fn search_returns_compact_highlight_ranges() {
    let mut engine = SearchEngine::new();
    engine.set_max_results(10);
    let store_id = engine.create_datastore("fuzzy");
    let session_id = engine.create_session(&store_id);
    engine.upsert_record(&store_id, "0", "folder/foo bar.md");

    let results = engine.search_session(&session_id, "foo");
    assert_eq!(results.length(), 1);

    let (_, _, ranges) = decode_store_result(&results.get(0));
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
    assert!(ranges.is_empty());
    assert_eq!(decode_matched_terms(&results.get(0)), vec!["apple", "pie"]);
}

#[wasm_bindgen_test]
fn full_text_datastore_expands_fuzzy_query_terms_through_word_catalog() {
    let mut engine = SearchEngine::new();
    engine.set_max_results(10);
    let store_id = engine.create_datastore("fullText");
    let session_id = engine.create_session(&store_id);

    engine.upsert_record(&store_id, "a", "Apple pie with apple slices");
    engine.upsert_record(&store_id, "b", "Banana tart");

    let results = engine.search_session(&session_id, "apl sli");

    assert_eq!(results.length(), 1);
    let (id, score, ranges) = decode_store_result(&results.get(0));
    assert_eq!(id, "a");
    assert!(score > 0);
    assert!(ranges.is_empty());
    assert_eq!(
        decode_matched_terms(&results.get(0)),
        vec!["apple", "slices"]
    );
}
