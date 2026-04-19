//! Crash-loop gate per MEGAPLAN §4.5: three consecutive failures within 60s
//! marks a language "unhealthy" for the process lifetime. Simple sliding
//! window keyed by language name.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const WINDOW: Duration = Duration::from_secs(60);
const TRIPS: usize = 3;

struct Window {
    failures: Vec<Instant>,
    permanently_unhealthy: bool,
}

impl Window {
    fn new() -> Self {
        Self {
            failures: Vec::with_capacity(TRIPS),
            permanently_unhealthy: false,
        }
    }
}

pub struct HealthTracker {
    windows: Mutex<HashMap<String, Window>>,
}

impl Default for HealthTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl HealthTracker {
    pub fn new() -> Self {
        Self {
            windows: Mutex::new(HashMap::new()),
        }
    }

    pub fn is_healthy(&self, language: &str) -> bool {
        let guard = self.windows.lock().expect("poisoned");
        match guard.get(language) {
            Some(w) => !w.permanently_unhealthy,
            None => true,
        }
    }

    pub fn record_failure(&self, language: &str) {
        let mut guard = self.windows.lock().expect("poisoned");
        let window = guard
            .entry(language.to_string())
            .or_insert_with(Window::new);
        let now = Instant::now();
        window.failures.retain(|t| now.duration_since(*t) <= WINDOW);
        window.failures.push(now);
        if window.failures.len() >= TRIPS {
            window.permanently_unhealthy = true;
        }
    }

    pub fn record_success(&self, language: &str) {
        let mut guard = self.windows.lock().expect("poisoned");
        if let Some(window) = guard.get_mut(language) {
            window.failures.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_failures_marks_unhealthy() {
        let tracker = HealthTracker::new();
        assert!(tracker.is_healthy("rust"));
        tracker.record_failure("rust");
        tracker.record_failure("rust");
        assert!(tracker.is_healthy("rust"));
        tracker.record_failure("rust");
        assert!(!tracker.is_healthy("rust"));
    }

    #[test]
    fn success_resets_window() {
        let tracker = HealthTracker::new();
        tracker.record_failure("rust");
        tracker.record_failure("rust");
        tracker.record_success("rust");
        tracker.record_failure("rust");
        assert!(tracker.is_healthy("rust"));
    }
}
