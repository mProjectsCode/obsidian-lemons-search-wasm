use std::collections::HashMap;

use wasm_bindgen::{prelude::*, JsValue};
use web_sys::js_sys;

use crate::{
    datastore::{Datastore, DatastoreKind, SearchSession},
    utils::StoreHealth,
    DEFAULT_MAX_RESULTS,
};

#[wasm_bindgen]
/// JavaScript-facing owner for search datastores and per-client sessions.
///
/// Store and session identifiers are opaque strings so the TypeScript plugin
/// can manage multiple indexes without holding Rust references across wasm.
pub struct SearchEngine {
    /// Active indexes keyed by generated `store:*` identifiers.
    stores: HashMap<String, Datastore>,
    /// Search sessions keyed by generated `session:*` identifiers.
    sessions: HashMap<String, SearchSession>,
    max_results: usize,
    next_store_id: u32,
    next_session_id: u32,
}

impl Default for SearchEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[wasm_bindgen]
impl SearchEngine {
    #[wasm_bindgen(constructor)]
    /// Creates an empty engine with the default result cap.
    pub fn new() -> Self {
        SearchEngine {
            stores: HashMap::new(),
            sessions: HashMap::new(),
            max_results: DEFAULT_MAX_RESULTS,
            next_store_id: 1,
            next_session_id: 1,
        }
    }

    /// Sets the maximum number of results returned by every search call.
    pub fn set_max_results(&mut self, max_results: usize) {
        self.max_results = max_results.max(1);
    }

    /// Creates a datastore and returns its opaque identifier.
    pub fn create_datastore(&mut self, kind: &str) -> String {
        let id = format!("store:{}", self.next_store_id);
        self.next_store_id += 1;

        self.stores
            .insert(id.clone(), Datastore::new(DatastoreKind::parse(kind)));
        id
    }

    /// Removes a datastore and closes every session attached to it.
    pub fn destroy_datastore(&mut self, store_id: &str) {
        self.stores.remove(store_id);
        self.sessions
            .retain(|_, session| session.store_id.as_str() != store_id);
    }

    /// Deletes all records from a datastore while keeping its identifier alive.
    pub fn clear_datastore(&mut self, store_id: &str) {
        if let Some(store) = self.stores.get_mut(store_id) {
            store.clear();
        }
        self.reset_sessions_for_store(store_id);
    }

    /// Starts a bulk rebuild by clearing the datastore and enabling faster
    /// batched ingestion where supported by the underlying store.
    pub fn begin_bulk_load(&mut self, store_id: &str) {
        if let Some(store) = self.stores.get_mut(store_id) {
            store.begin_bulk_load();
        }
        self.reset_sessions_for_store(store_id);
    }

    /// Finishes a bulk rebuild and restores query-ready datastore invariants.
    pub fn finish_bulk_load(&mut self, store_id: &str) {
        if let Some(store) = self.stores.get_mut(store_id) {
            store.finish_bulk_load();
        }
        self.reset_sessions_for_store(store_id);
    }

    /// Inserts or replaces a single record in the target datastore.
    pub fn upsert_record(&mut self, store_id: &str, id: &str, text: &str) {
        if let Some(store) = self.stores.get_mut(store_id) {
            store.upsert(id.to_string(), text.to_string());
        }
        self.reset_sessions_for_store(store_id);
    }

    /// Inserts or replaces records from a JavaScript array of `{ id, text }`.
    ///
    /// Malformed entries are skipped so one bad item does not reject the whole
    /// batch coming from the plugin process.
    pub fn upsert_records(&mut self, store_id: &str, records: JsValue) {
        let Some(store) = self.stores.get_mut(store_id) else {
            return;
        };

        for record in js_sys::Array::from(&records).iter() {
            let Some((id, text)) = read_record(&record) else {
                continue;
            };
            store.upsert(id, text);
        }

        self.reset_sessions_for_store(store_id);
    }

    /// Deletes records by id and resets affected incremental sessions.
    pub fn delete_records(&mut self, store_id: &str, ids: Vec<String>) {
        if let Some(store) = self.stores.get_mut(store_id) {
            for id in ids {
                store.delete(&id);
            }
        }
        self.reset_sessions_for_store(store_id);
    }

    /// Deletes every record whose id starts with `prefix`.
    pub fn delete_records_by_prefix(&mut self, store_id: &str, prefix: &str) {
        if let Some(store) = self.stores.get_mut(store_id) {
            store.delete_by_prefix(prefix);
        }
        self.reset_sessions_for_store(store_id);
    }

    /// Opens a search session for a datastore and returns its opaque id.
    pub fn create_session(&mut self, store_id: &str) -> String {
        let Some(store) = self.stores.get(store_id) else {
            return String::new();
        };

        let id = format!("session:{}", self.next_session_id);
        self.next_session_id += 1;
        self.sessions.insert(
            id.clone(),
            SearchSession::new(store_id.to_string(), store.kind()),
        );
        id
    }

    /// Closes a search session.
    pub fn close_session(&mut self, session_id: &str) {
        self.sessions.remove(session_id);
    }

    /// Searches through an existing session and returns a JavaScript result array.
    pub fn search_session(&mut self, session_id: &str, query: &str) -> js_sys::Array {
        let Some(session) = self.sessions.get_mut(session_id) else {
            return js_sys::Array::new();
        };
        let Some(store) = self.stores.get(&session.store_id) else {
            return js_sys::Array::new();
        };

        let js_results = js_sys::Array::new();
        for result in store.search(session, query, self.max_results) {
            js_results.push(&result.into_js_object());
        }
        js_results
    }

    /// Returns diagnostic information for a datastore, or a missing marker.
    pub fn datastore_health(&self, store_id: &str) -> JsValue {
        let Some(store) = self.stores.get(store_id) else {
            return StoreHealth::missing().into_js_object().into();
        };
        store.health().into_js_object().into()
    }
}

impl SearchEngine {
    /// Invalidates per-session caches after records in a store are mutated.
    fn reset_sessions_for_store(&mut self, store_id: &str) {
        for session in self.sessions.values_mut() {
            if session.store_id == store_id {
                session.reset();
            }
        }
    }
}

/// Extracts a datastore record from a JavaScript value.
fn read_record(record: &JsValue) -> Option<(String, String)> {
    let id = js_sys::Reflect::get(record, &"id".into())
        .ok()?
        .as_string()?;
    let text = js_sys::Reflect::get(record, &"text".into())
        .ok()?
        .as_string()?;

    Some((id, text))
}
