mod datastore;
mod engine;
mod full_text;
mod fuzzy;
mod legacy_search;
mod utils;

use wasm_bindgen::prelude::*;

pub use engine::SearchEngine;
pub use legacy_search::Search;

const DEFAULT_MAX_RESULTS: usize = 200;

#[wasm_bindgen]
pub fn setup() {
    console_error_panic_hook::set_once();
}
