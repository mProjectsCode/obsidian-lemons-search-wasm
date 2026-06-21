use std::{cmp::Ordering, collections::BinaryHeap};

/// Score/index pair with reversed ordering for bounded min-heap behavior.
#[derive(Debug, Clone, Copy, Eq)]
pub(crate) struct ScoredRecordIndex {
    score: u32,
    record_index: usize,
}

impl ScoredRecordIndex {
    /// Creates a scored record-index entry.
    pub(crate) fn new(score: u32, record_index: usize) -> Self {
        ScoredRecordIndex {
            score,
            record_index,
        }
    }

    #[inline]
    /// Returns the match score.
    pub(crate) fn score(&self) -> u32 {
        self.score
    }

    #[inline]
    /// Returns the associated record-table index.
    pub(crate) fn record_index(&self) -> usize {
        self.record_index
    }

    /// Maintains a heap containing only the highest scoring entries.
    pub(crate) fn push_top_score(
        heap: &mut BinaryHeap<Self>,
        scored_record: Self,
        max_results: usize,
    ) {
        if heap.len() < max_results {
            heap.push(scored_record);
            return;
        }

        let Some(worst_record) = heap.peek() else {
            return;
        };

        if scored_record.score() > worst_record.score() {
            heap.pop();
            heap.push(scored_record);
        }
    }
}

impl PartialEq for ScoredRecordIndex {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.score == other.score
    }
}

impl PartialOrd for ScoredRecordIndex {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ScoredRecordIndex {
    #[inline]
    fn cmp(&self, other: &Self) -> Ordering {
        self.score.cmp(&other.score).reverse()
    }
}
