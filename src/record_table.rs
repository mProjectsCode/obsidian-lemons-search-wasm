use std::{collections::HashMap, rc::Rc};

/// A record stored in a sparse table with a stable external id.
pub(crate) struct StoredRecord<T> {
    pub(crate) id: Rc<str>,
    pub(crate) value: T,
}

/// Sparse record storage shared by search datastores.
///
/// Deletions leave reusable slots so numeric record indices stay stable while
/// avoiding unbounded tombstone growth across update/delete cycles.
pub(crate) struct RecordTable<T> {
    records: Vec<Option<StoredRecord<T>>>,
    id_to_index: HashMap<Rc<str>, usize>,
    free_indices: Vec<usize>,
    live_record_count: usize,
}

impl<T> RecordTable<T> {
    pub(crate) fn new() -> Self {
        RecordTable {
            records: Vec::new(),
            id_to_index: HashMap::new(),
            free_indices: Vec::new(),
            live_record_count: 0,
        }
    }

    /// Inserts or replaces a record, returning its slot and the replaced value.
    pub(crate) fn upsert(&mut self, id: String, value: T) -> (usize, Option<StoredRecord<T>>) {
        let (idx, replaced) = if let Some(idx) = self.id_to_index.remove(id.as_str()) {
            let replaced = self.records.get_mut(idx).and_then(Option::take);
            if replaced.is_some() {
                self.live_record_count = self.live_record_count.saturating_sub(1);
            }
            (idx, replaced)
        } else if let Some(idx) = self.free_indices.pop() {
            (idx, None)
        } else {
            let idx = self.records.len();
            self.records.push(None);
            (idx, None)
        };

        let id = Rc::<str>::from(id);
        self.id_to_index.insert(Rc::clone(&id), idx);
        self.records[idx] = Some(StoredRecord { id, value });
        self.live_record_count += 1;

        (idx, replaced)
    }

    /// Deletes a live record and makes its slot available for reuse.
    pub(crate) fn delete(&mut self, id: &str) -> Option<(usize, StoredRecord<T>)> {
        let idx = self.id_to_index.remove(id)?;
        let record = self.records.get_mut(idx).and_then(Option::take)?;

        self.live_record_count = self.live_record_count.saturating_sub(1);
        self.free_indices.push(idx);

        Some((idx, record))
    }

    pub(crate) fn clear(&mut self) {
        self.records.clear();
        self.id_to_index.clear();
        self.free_indices.clear();
        self.live_record_count = 0;
    }

    pub(crate) fn get(&self, idx: usize) -> Option<&StoredRecord<T>> {
        self.records.get(idx).and_then(Option::as_ref)
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &StoredRecord<T>> {
        self.records.iter().filter_map(Option::as_ref)
    }

    pub(crate) fn slot_count(&self) -> usize {
        self.records.len()
    }

    pub(crate) fn live_record_count(&self) -> usize {
        self.live_record_count
    }

    pub(crate) fn tombstone_count(&self) -> usize {
        self.records.len().saturating_sub(self.live_record_count)
    }

    pub(crate) fn record_ids(&self) -> Vec<String> {
        self.id_to_index
            .keys()
            .map(|id| id.as_ref().to_string())
            .collect()
    }
}
