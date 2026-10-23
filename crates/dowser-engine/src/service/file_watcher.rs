//! Background file watcher: auto-reindexes workspaces after ingest.

use rustc_hash::{FxHashMap, FxHashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use notify_debouncer_full::{DebounceEventResult, new_debouncer, notify::RecursiveMode};

use crate::config::AppConfig;
use crate::indexing::{Indexer, code_index};
use dowser_storage::storage::IndexStore;

struct WatchHandle {
    stop: mpsc::Sender<Option<Vec<notify_debouncer_full::DebouncedEvent>>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for WatchHandle {
    fn drop(&mut self) {
        let _ = self.stop.send(None);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
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
        let (tx, rx) = mpsc::channel();
        let stop = tx.clone();
        let handle = std::thread::Builder::new()
            .name(format!("watch-{repo_id}"))
            .spawn(move || {
                if let Err(e) = run_watcher(&repo_id_owned, &root, &config, &indexer, tx, rx) {
                    tracing::warn!(repo = %repo_id_owned, error = %e, "file watcher exited");
                }
            })
            .expect("failed to spawn watcher thread");

        map.insert(
            repo_id.to_string(),
            WatchHandle {
                stop,
                thread: Some(handle),
            },
        );
        tracing::info!(repo = %repo_id, path = %root_display, "started background file watcher");
    }
}

fn run_watcher(
    repo_id: &str,
    root: &Path,
    config: &AppConfig,
    indexer: &Arc<Indexer>,
    tx: mpsc::Sender<Option<Vec<notify_debouncer_full::DebouncedEvent>>>,
    rx: mpsc::Receiver<Option<Vec<notify_debouncer_full::DebouncedEvent>>>,
) -> anyhow::Result<()> {
    let store = IndexStore::open(config.repo_db_path(repo_id))?;
    let storage_dir = config.index_root();

    let debounce = Duration::from_millis(500);
    let mut debouncer = new_debouncer(debounce, None, move |result: DebounceEventResult| {
        if let Ok(events) = result {
            let _ = tx.send(Some(events));
        }
    })?;

    debouncer.watch(root, RecursiveMode::Recursive)?;

    while let Ok(Some(events)) = rx.recv() {
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

    debouncer.stop();
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
            if dowser_storage::db::SKIP_DIRS.iter().any(|s| *s == name) {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_watcher_create_drop_releases_worker_resources() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().join("repo");
        std::fs::create_dir(&root).unwrap();
        let config = AppConfig::new(workspace.path().join("index"));
        config.ensure_layout().unwrap();
        let _store = IndexStore::open(config.repo_db_path("lifecycle")).unwrap();
        let indexer = Arc::new(Indexer::new());
        for _ in 0..50 {
            let manager = WatcherManager::new();
            manager.ensure_watching("lifecycle", root.clone(), config.clone(), indexer.clone());
            drop(manager);
            assert_eq!(Arc::strong_count(&indexer), 1, "watcher outlived its owner");
        }
    }
}
