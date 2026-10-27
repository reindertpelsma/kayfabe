//! Names and reverse references used by the RM policy retirement paths.
use std::collections::{BTreeMap, BTreeSet};
pub(crate) type Key = (u32, u32);

#[derive(Default)]
pub(crate) struct AliasIndex {
    names: BTreeMap<Key, Key>,
    users: BTreeMap<Key, BTreeSet<Key>>,
}
impl AliasIndex {
    pub(crate) fn get(&self, name: &Key) -> Option<&Key> {
        self.names.get(name)
    }
    pub(crate) fn has_refs(&self, original: &Key) -> bool {
        self.users.contains_key(original)
    }
    pub(crate) fn users(&self, original: &Key) -> Vec<Key> {
        self.users
            .get(original)
            .map_or_else(Vec::new, |names| names.iter().copied().collect())
    }
    pub(crate) fn insert(&mut self, name: Key, original: Key) -> Option<Key> {
        let old = self.remove(name);
        self.names.insert(name, original);
        self.users.entry(original).or_default().insert(name);
        old
    }
    pub(crate) fn remove(&mut self, name: Key) -> Option<Key> {
        let original = self.names.remove(&name)?;
        if let Some(users) = self.users.get_mut(&original) {
            users.remove(&name);
            if users.is_empty() {
                self.users.remove(&original);
            }
        }
        Some(original)
    }
    pub(crate) fn remove_for_free(&mut self, client: u32, object: u32) -> BTreeSet<Key> {
        let keys = free_keys(&self.names, client, object);
        keys.into_iter()
            .filter_map(|key| self.remove(key))
            .collect()
    }
}
pub(crate) fn free_keys<T>(map: &BTreeMap<Key, T>, client: u32, object: u32) -> Vec<Key> {
    if client == object {
        map.range((client, 0)..=(client, u32::MAX))
            .map(|(key, _)| *key)
            .collect()
    } else if map.contains_key(&(client, object)) {
        vec![(client, object)]
    } else {
        Vec::new()
    }
}
pub(crate) fn remove_for_free<T>(map: &mut BTreeMap<Key, T>, client: u32, object: u32) {
    for key in free_keys(map, client, object) {
        map.remove(&key);
    }
}
