//! Small in-memory LRU for remuxed archive playback responses.
//!
//! Archive segments are immutable, so caching the fMP4 result avoids repeated
//! RustFS downloads and FFmpeg runs when operators scrub/replay a segment.

use std::{
    collections::VecDeque,
    sync::{Mutex, OnceLock},
};

pub struct PlaybackCache {
    max_items: usize,
    max_bytes: usize,
    entries: Mutex<VecDeque<(String, Vec<u8>)>>,
}

impl PlaybackCache {
    pub fn new(max_items: usize, max_bytes: usize) -> Self {
        Self {
            max_items: max_items.max(1),
            max_bytes: max_bytes.max(1),
            entries: Mutex::new(VecDeque::new()),
        }
    }

    pub fn get(&self, key: &str) -> Option<Vec<u8>> {
        let Ok(mut entries) = self.entries.lock() else {
            return None;
        };
        let position = entries.iter().position(|(candidate, _)| candidate == key)?;
        let entry = entries.remove(position)?;
        let bytes = entry.1.clone();
        entries.push_front(entry);
        Some(bytes)
    }

    pub fn put(&self, key: String, bytes: Vec<u8>) {
        if bytes.len() > self.max_bytes {
            return;
        }
        let Ok(mut entries) = self.entries.lock() else {
            return;
        };
        entries.retain(|(candidate, _)| candidate != &key);
        entries.push_front((key, bytes));
        let mut total: usize = entries.iter().map(|(_, value)| value.len()).sum();
        while entries.len() > self.max_items || total > self.max_bytes {
            let Some((_, removed)) = entries.pop_back() else {
                break;
            };
            total = total.saturating_sub(removed.len());
        }
    }

    pub fn len(&self) -> usize {
        self.entries
            .lock()
            .map(|entries| entries.len())
            .unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

pub fn global() -> &'static PlaybackCache {
    static CACHE: OnceLock<PlaybackCache> = OnceLock::new();
    CACHE.get_or_init(|| {
        let items = std::env::var("TSP_ARCHIVE_PLAYBACK_CACHE_ITEMS")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|value| *value >= 1)
            .unwrap_or(8);
        let bytes = std::env::var("TSP_ARCHIVE_PLAYBACK_CACHE_BYTES")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|value| *value >= 1024 * 1024)
            .unwrap_or(64 * 1024 * 1024);
        PlaybackCache::new(items, bytes)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_is_lru_by_item_count() {
        let cache = PlaybackCache::new(2, 1024);
        cache.put("a".to_string(), vec![1]);
        cache.put("b".to_string(), vec![2]);
        assert_eq!(cache.get("a"), Some(vec![1]));
        cache.put("c".to_string(), vec![3]);
        assert_eq!(cache.len(), 2);
        assert!(cache.get("a").is_some());
        assert!(cache.get("b").is_none());
        assert!(cache.get("c").is_some());
    }

    #[test]
    fn cache_evicts_by_bytes_and_skips_oversize() {
        let cache = PlaybackCache::new(10, 5);
        cache.put("a".to_string(), vec![0; 3]);
        cache.put("b".to_string(), vec![0; 3]);
        assert!(cache.get("a").is_none());
        assert!(cache.get("b").is_some());
        cache.put("huge".to_string(), vec![0; 6]);
        assert!(cache.get("huge").is_none());
        assert_eq!(cache.len(), 1);
    }
}
