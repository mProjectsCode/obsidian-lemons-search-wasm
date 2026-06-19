use crate::{
    full_text::FullTextDatastore,
    fuzzy::{FuzzyDatastore, FuzzySessionState},
    utils::{StoreHealth, StoreSearchResult},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DatastoreKind {
    Fuzzy,
    FullText,
}

impl DatastoreKind {
    pub(crate) fn parse(kind: &str) -> Self {
        match kind {
            "fullText" => DatastoreKind::FullText,
            _ => DatastoreKind::Fuzzy,
        }
    }

    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            DatastoreKind::Fuzzy => "fuzzy",
            DatastoreKind::FullText => "fullText",
        }
    }
}

pub(crate) enum Datastore {
    Fuzzy(FuzzyDatastore),
    FullText(FullTextDatastore),
}

impl Datastore {
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

pub(crate) struct SearchSession {
    pub(crate) store_id: String,
    kind: DatastoreKind,
    fuzzy: Option<FuzzySessionState>,
}

impl SearchSession {
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

    pub(crate) fn reset(&mut self) {
        if let Some(state) = self.fuzzy.as_mut() {
            state.reset();
        }
    }
}
