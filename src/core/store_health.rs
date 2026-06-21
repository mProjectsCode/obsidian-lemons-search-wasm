use web_sys::js_sys;

/// Diagnostic snapshot for a datastore.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DatastoreHealth {
    pub(crate) exists: bool,
    pub(crate) kind: String,
    pub(crate) live_records: usize,
    pub(crate) tombstones: usize,
    pub(crate) record_ids: Vec<String>,
    pub(crate) posting_terms: usize,
    pub(crate) posting_occurrences: usize,
}

impl DatastoreHealth {
    /// Builds a health snapshot for an existing datastore.
    pub(crate) fn new(
        kind: &str,
        live_records: usize,
        tombstones: usize,
        mut record_ids: Vec<String>,
        posting_terms: usize,
        posting_occurrences: usize,
    ) -> Self {
        record_ids.sort();
        DatastoreHealth {
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
    pub(crate) fn missing() -> Self {
        DatastoreHealth {
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
    pub(crate) fn into_js_object(self) -> js_sys::Object {
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
