//! Background file watcher: auto-reindexes workspaces after ingest.

use rustc_hash::{FxHashMap, FxHashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use notify_debouncer_full::{DebounceEventResult, new_debouncer, notify::RecursiveMode};

use crate::config::AppConfig;
use crate::indexing::{Indexer, code_index};
use codesight_storage::storage::IndexStore;

struct WatchHandle {
    _thread: std::thread::JoinHandle<()>,
}

pub(crate) struct WatcherManager {
    active: Mutex<FxHashMap<String, WatchHandle>>,
}

impl WatcherManager {
    pub fn new() -> Self {
        Self {
            active: Mutex::new(FxHashMap::default()),
        }
    }

    pub fn ensure_watching(
        &self,
        repo_id: &str,
        root: PathBuf,
        config: AppConfig,
        indexer: Arc<Indexer>,
    ) {
        let mut map = self.active.lock().unwrap();
        if map.contains_key(repo_id) {
            return;
        }

        let repo_id_owned = repo_id.to_string();
        let root_display = root.display().to_string();
        let handle = std::thread::Builder::new()
            .name(format!("watch-{repo_id}"))
            .spawn(move || {
                if let Err(e) = run_watcher(&repo_id_owned, &root, &config, &indexer) {
                    tracing::warn!(repo = %repo_id_owned, error = %e, "file watcher exited");
                }
            })
            .expect("failed to spawn watcher thread");

        map.insert(repo_id.to_string(), WatchHandle { _thread: handle });
        tracing::info!(repo = %repo_id, path = %root_display, "started background file watcher");
    }
}

fn run_watcher(
    repo_id: &str,
    root: &Path,
    config: &AppConfig,
    indexer: &Arc<Indexer>,
) -> anyhow::Result<()> {
    let store = IndexStore::open(config.repo_db_path(repo_id))?;
    let storage_dir = config.index_root();

    let (tx, rx) = mpsc::channel();
    let debounce = Duration::from_millis(500);
    let mut debouncer = new_debouncer(debounce, None, move |result: DebounceEventResult| {
        if let Ok(events) = result {
            let _ = tx.send(events);
        }
    })?;

    debouncer.watch(root, RecursiveMode::Recursive)?;

    while let Ok(events) = rx.recv() {
        let changed = collect_changed_paths(&events, root, storage_dir);
        if changed.is_empty() {
            continue;
        }
        let start = std::time::Instant::now();
        match code_index::reindex_files(config, root, repo_id, &store, indexer, &changed) {
            Ok(stats) => {
                tracing::debug!(
                    repo = %repo_id,
                    files = stats.files_processed,
                    removed = stats.files_removed,
                    symbols = stats.symbols_extracted,
                    elapsed_ms = start.elapsed().as_millis() as u64,
                    "background delta reindex"
                );
            }
            Err(err) => {
                tracing::warn!(repo = %repo_id, error = %err, "background delta reindex failed");
            }
        }
    }

    Ok(())
}

fn collect_changed_paths(
    events: &[notify_debouncer_full::DebouncedEvent],
    root: &Path,
    storage_dir: &Path,
) -> Vec<PathBuf> {
    let mut paths = FxHashSet::default();
    for event in events {
        use notify::EventKind;
        match &event.kind {
            EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) => {
                for p in &event.paths {
                    if p.starts_with(root)
                        && !is_in_skip_dir(p, root)
                        && !p.starts_with(storage_dir)
                    {
                        paths.insert(p.clone());
                    }
                }
            }
            _ => {}
        }
    }
    paths.into_iter().collect()
}

fn is_in_skip_dir(path: &Path, root: &Path) -> bool {
    if let Ok(rel) = path.strip_prefix(root) {
        for component in rel.components() {
            let name = component.as_os_str().to_string_lossy();
            if codesight_storage::db::SKIP_DIRS.iter().any(|s| *s == name) {
                return true;
            }
        }
    }
    false
}
