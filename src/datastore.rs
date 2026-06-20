use crate::{
    full_text::FullTextDatastore,
    fuzzy::{FuzzyDatastore, FuzzySessionState},
    utils::{StoreHealth, StoreSearchResult},
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

    pub(crate) fn clear(&mut self) {
        match self {
            Datastore::Fuzzy(store) => store.clear(),
            Datastore::FullText(store) => store.clear(),
        }
    }

    pub(crate) fn health(&self) -> StoreHealth {
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
    ) -> Vec<StoreSearchResult> {
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

/// Per-consumer search state tied to a single datastore.
///
/// Fuzzy search keeps incremental narrowing state here so separate UI clients
/// can search the same store without sharing query history.
pub(crate) struct SearchSession {
    pub(crate) store_id: String,
    kind: DatastoreKind,
    fuzzy: Option<FuzzySessionState>,
}

impl SearchSession {
    /// Creates a session with any implementation-specific state required by the
    /// target datastore.
    pub(crate) fn new(store_id: String, kind: DatastoreKind) -> Self {
        SearchSession {
            store_id,
            kind,
            fuzzy: match kind {
                DatastoreKind::Fuzzy => Some(FuzzySessionState::default()),
                DatastoreKind::FullText => None,
            },
        }
    }

    /// Clears cached query state after the underlying datastore changes.
    pub(crate) fn reset(&mut self) {
        if let Some(state) = self.fuzzy.as_mut() {
            state.reset();
        }
    }
}
