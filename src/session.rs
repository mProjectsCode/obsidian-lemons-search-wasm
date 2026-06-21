use crate::stores::{fuzzy::FuzzySessionState, DatastoreKind};

/// Per-consumer search state tied to a single datastore.
///
/// Fuzzy search keeps incremental narrowing state here so separate UI clients
/// can search the same store without sharing query history.
pub(crate) struct SearchSession {
    pub(crate) store_id: String,
    pub(crate) kind: DatastoreKind,
    pub(crate) fuzzy: Option<FuzzySessionState>,
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
