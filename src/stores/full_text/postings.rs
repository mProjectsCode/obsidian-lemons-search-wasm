use super::{model::Posting, types::RecordIdx};

/// Inserts or replaces a posting while preserving record-index sort order.
pub(super) fn insert_posting(
    postings: &mut Vec<Posting>,
    record_idx: RecordIdx,
    term_frequency: u32,
) {
    if postings
        .last()
        .is_none_or(|posting| posting.record_idx < record_idx)
    {
        postings.push(Posting {
            record_idx,
            term_frequency,
        });
        return;
    }

    match postings.binary_search_by_key(&record_idx, |posting| posting.record_idx) {
        Ok(idx) => postings[idx].term_frequency = term_frequency,
        Err(idx) => postings.insert(
            idx,
            Posting {
                record_idx,
                term_frequency,
            },
        ),
    }
}
