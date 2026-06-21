pub(crate) mod full_text;
pub(crate) mod fuzzy;

use crate::{
    core::{search_result::SearchResult, store_health::DatastoreHealth},
    session::SearchSession,
    stores::{full_text::FullTextDatastore, fuzzy::FuzzyDatastore},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Selects the search implementation backing a datastore.
pub(crate) enum DatastoreKind {
    Fuzzy,
    FullText,
}

impl DatastoreKind {
    /// Parses the JavaScript-facing store kind, defaulting to fuzzy search for
    /// unknown values to preserve the legacy behavior.
    pub(crate) fn parse(kind: &str) -> Self {
        match kind {
            "fullText" => DatastoreKind::FullText,
            _ => DatastoreKind::Fuzzy,
        }
    }

    /// Returns the stable string used in diagnostics and health checks.
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            DatastoreKind::Fuzzy => "fuzzy",
            DatastoreKind::FullText => "fullText",
        }
    }
}

/// Type-erased datastore wrapper used by the wasm-facing engine.
pub(crate) enum Datastore {
    Fuzzy(FuzzyDatastore),
    FullText(FullTextDatastore),
}

impl Datastore {
    /// Builds a concrete datastore for the requested implementation.
    pub(crate) fn new(kind: DatastoreKind) -> Self {
        match kind {
            DatastoreKind::Fuzzy => Datastore::Fuzzy(FuzzyDatastore::new()),
            DatastoreKind::FullText => Datastore::FullText(FullTextDatastore::new()),
        }
    }

    pub(crate) fn kind(&self) -> DatastoreKind {
        match self {
            Datastore::Fuzzy(_) => DatastoreKind::Fuzzy,
            Datastore::FullText(_) => DatastoreKind::FullText,
        }
    }

    pub(crate) fn upsert(&mut self, id: String, text: String) {
        match self {
            Datastore::Fuzzy(store) => store.upsert(id, text),
            Datastore::FullText(store) => store.upsert(id, text),
        }
    }

    pub(crate) fn delete(&mut self, id: &str) {
        match self {
            Datastore::Fuzzy(store) => store.delete(id),
            Datastore::FullText(store) => store.delete(id),
        }
    }

    pub(crate) fn delete_by_prefix(&mut self, prefix: &str) {
        match self {
            Datastore::Fuzzy(store) => store.delete_by_prefix(prefix),
            Datastore::FullText(store) => store.delete_by_prefix(prefix),
        }
    }

    pub(crate) fn clear(&mut self) {
        match self {
            Datastore::Fuzzy(store) => store.clear(),
            Datastore::FullText(store) => store.clear(),
        }
    }

    pub(crate) fn begin_bulk_load(&mut self) {
        match self {
            Datastore::Fuzzy(store) => store.clear(),
            Datastore::FullText(store) => store.begin_bulk_load(),
        }
    }

    pub(crate) fn set_full_text_fuzzy_search(&mut self, enabled: bool) {
        if let Datastore::FullText(store) = self {
            store.set_fuzzy_search(enabled);
        }
    }

    pub(crate) fn finish_bulk_load(&mut self) {
        match self {
            Datastore::Fuzzy(_) => {}
            Datastore::FullText(store) => store.finish_bulk_load(),
        }
    }

    pub(crate) fn health(&self) -> DatastoreHealth {
        match self {
            Datastore::Fuzzy(store) => store.health(),
            Datastore::FullText(store) => store.health(),
        }
    }

    /// Dispatches a search to the matching datastore and session state.
    ///
    /// A session created for one store kind is not reusable for another kind;
    /// mismatches return no results instead of panicking.
    pub(crate) fn search(
        &self,
        session: &mut SearchSession,
        query: &str,
        max_results: usize,
    ) -> Vec<SearchResult> {
        match (self, session.kind) {
            (Datastore::Fuzzy(store), DatastoreKind::Fuzzy) => {
                let Some(state) = session.fuzzy.as_mut() else {
                    return Vec::new();
                };
                store.search(state, query, max_results)
            }
            (Datastore::FullText(store), DatastoreKind::FullText) => {
                store.search(query, max_results)
            }
            _ => Vec::new(),
        }
    }
}
