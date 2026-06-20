mod datastore;
mod engine;
mod full_text;
mod fuzzy;
mod record_table;
mod utils;

use wasm_bindgen::prelude::*;

pub use engine::SearchEngine;

const DEFAULT_MAX_RESULTS: usize = 200;

/// Installs a panic hook that forwards Rust panics to the browser console.
#[wasm_bindgen]
pub fn setup() {
    console_error_panic_hook::set_once();
}
