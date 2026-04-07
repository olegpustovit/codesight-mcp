//! Session-level LRU result cache keyed by (tool_name, source_id, specific_key).

use std::collections::HashMap;
use std::sync::Mutex;

/// Key for a cached result entry.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CacheKey {
    pub tool_name: String,
    pub source_id: String,
    pub specific_key: String,
}

/// Per-tool hit/miss statistics.
#[derive(Clone, Debug, Default)]
pub struct ToolStats {
    pub hits: u64,
    pub misses: u64,
}

impl ToolStats {
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64
        }
    }
}

/// Aggregate cache statistics.
#[derive(Clone, Debug, Default)]
pub struct CacheStats {
    pub total_hits: u64,
    pub total_misses: u64,
    pub by_tool: HashMap<String, ToolStats>,
}

/// Snapshot of cache statistics returned by [`ResultCache::stats`].
#[derive(Clone, Debug)]
pub struct CacheStatsSnapshot {
    pub total_hits: u64,
    pub total_misses: u64,
    pub hit_rate: f64,
    pub cached_entries: usize,
    pub by_tool: HashMap<String, ToolStats>,
}

/// Internal entry stored in the LRU map.
struct Entry {
    key: CacheKey,
    value: serde_json::Value,
    /// Monotonically increasing counter — higher = more recently used.
    generation: u64,
}

/// A thread-safe LRU result cache.
pub struct ResultCache {
    /// All entries indexed by key.
    entries: Mutex<HashMap<CacheKey, Entry>>,
    max_size: usize,
    stats: Mutex<CacheStats>,
    /// Monotonic generation counter (inside entries lock).
    generation: Mutex<u64>,
}

impl ResultCache {
    /// Create a new cache with the given maximum capacity.
    pub fn new(max_size: usize) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            max_size,
            stats: Mutex::new(CacheStats::default()),
            generation: Mutex::new(0),
        }
    }

    fn next_gen(&self) -> u64 {
        let mut g = self.generation.lock().unwrap();
        *g += 1;
        *g
    }

    /// Look up a cached value. Returns `Some(value)` on hit, `None` on miss.
    /// Updates LRU position on hit. Records hit/miss in stats.
    pub fn get(
        &self,
        tool: &str,
        source_id: &str,
        specific_key: &str,
    ) -> Option<serde_json::Value> {
        let key = CacheKey {
            tool_name: tool.to_string(),
            source_id: source_id.to_string(),
            specific_key: specific_key.to_string(),
        };

        let result = {
            let mut entries = self.entries.lock().unwrap();
            if let Some(entry) = entries.get_mut(&key) {
                let g = self.next_gen();
                entry.generation = g;
                Some(entry.value.clone())
            } else {
                None
            }
        };

        let mut stats = self.stats.lock().unwrap();
        if result.is_some() {
            stats.total_hits += 1;
            stats.by_tool.entry(tool.to_string()).or_default().hits += 1;
        } else {
            stats.total_misses += 1;
            stats.by_tool.entry(tool.to_string()).or_default().misses += 1;
        }

        result
    }

    /// Store a value in the cache. Evicts the least-recently-used entry if at capacity.
    pub fn put(&self, tool: &str, source_id: &str, specific_key: &str, value: serde_json::Value) {
        let key = CacheKey {
            tool_name: tool.to_string(),
            source_id: source_id.to_string(),
            specific_key: specific_key.to_string(),
        };

        let g = self.next_gen();
        let mut entries = self.entries.lock().unwrap();

        // If key already present, just update.
        if let Some(entry) = entries.get_mut(&key) {
            entry.value = value;
            entry.generation = g;
            return;
        }

        // Evict LRU if at capacity.
        if entries.len() >= self.max_size {
            let lru_key = entries
                .values()
                .min_by_key(|e| e.generation)
                .map(|e| e.key.clone());
            if let Some(k) = lru_key {
                entries.remove(&k);
            }
        }

        entries.insert(
            key.clone(),
            Entry {
                key,
                value,
                generation: g,
            },
        );
    }

    /// Invalidate cache entries. If `source_id` is `None`, all entries are evicted.
    /// If `Some(id)`, only entries for that source are evicted.
    /// Returns the number of entries evicted.
    pub fn invalidate(&self, source_id: Option<&str>) -> usize {
        let mut entries = self.entries.lock().unwrap();
        match source_id {
            None => {
                let count = entries.len();
                entries.clear();
                count
            }
            Some(sid) => {
                let before = entries.len();
                entries.retain(|k, _| k.source_id != sid);
                before - entries.len()
            }
        }
    }

    /// Return a snapshot of the current cache statistics.
    pub fn stats(&self) -> CacheStatsSnapshot {
        let stats = self.stats.lock().unwrap();
        let entries = self.entries.lock().unwrap();
        let total = stats.total_hits + stats.total_misses;
        CacheStatsSnapshot {
            total_hits: stats.total_hits,
            total_misses: stats.total_misses,
            hit_rate: if total == 0 {
                0.0
            } else {
                stats.total_hits as f64 / total as f64
            },
            cached_entries: entries.len(),
            by_tool: stats.by_tool.clone(),
        }
    }
}
