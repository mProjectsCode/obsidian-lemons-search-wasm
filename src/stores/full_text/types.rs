pub(super) type RecordIdx = u32;
pub(super) type TermId = u32;

/// Converts a sparse record vector index into the compact posting-list type.
pub(super) fn record_index_from_usize(index: usize) -> RecordIdx {
    u32::try_from(index).expect("full-text record count exceeded u32::MAX")
}

/// Converts a record token length into the compact per-record type.
pub(super) fn token_count_from_usize(count: usize) -> u32 {
    u32::try_from(count).expect("full-text record token count exceeded u32::MAX")
}

/// Converts a posting-list vector index into the compact term id type.
pub(super) fn term_id_from_usize(index: usize) -> TermId {
    u32::try_from(index).expect("full-text term count exceeded u32::MAX")
}
