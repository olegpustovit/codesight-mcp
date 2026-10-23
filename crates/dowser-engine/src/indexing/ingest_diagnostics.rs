//! Opt-in allocation-localization events; never includes source text.
use dowser_core::StoredSymbol;

#[derive(Clone, Copy, Default)]
pub(super) struct PayloadCounts {
    pub candidate_files: usize,
    pub input_bytes: u64,
    pub files: usize,
    pub source_bytes: usize,
    pub symbols: usize,
    pub body_bytes: usize,
    pub signature_bytes: usize,
}

impl PayloadCounts {
    pub fn add(&mut self, source: &str, symbols: &[StoredSymbol]) {
        self.files += 1;
        self.source_bytes += source.len();
        self.symbols += symbols.len();
        self.body_bytes += symbols
            .iter()
            .map(|symbol| symbol.source_text.len())
            .sum::<usize>();
        self.signature_bytes += symbols
            .iter()
            .map(|symbol| symbol.signature.len())
            .sum::<usize>();
    }
}

#[derive(Clone, Copy)]
pub(super) struct IngestDiagnostics(pub bool);

impl IngestDiagnostics {
    pub fn from_env() -> Self {
        Self(std::env::var("DOWSER_INGEST_DIAGNOSTICS").as_deref() == Ok("1"))
    }

    pub fn record(self, phase: &str, path: Option<&str>, counts: PayloadCounts) {
        if !self.0 {
            return;
        }
        let rss_bytes = std::fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|status| rss_bytes(&status));
        tracing::info!(
            phase,
            path,
            candidate_files = counts.candidate_files,
            input_bytes = counts.input_bytes,
            files = counts.files,
            retained_source_bytes = counts.source_bytes,
            symbols = counts.symbols,
            symbol_body_bytes = counts.body_bytes,
            symbol_signature_bytes = counts.signature_bytes,
            rss_bytes,
            "ingest diagnostic"
        );
    }
}

fn rss_bytes(status: &str) -> Option<usize> {
    status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))?
        .split_whitespace()
        .next()?
        .parse::<usize>()
        .ok()?
        .checked_mul(1024)
}

#[cfg(test)]
mod tests {
    #[test]
    fn linux_rss_is_converted_from_kib_or_left_unknown() {
        assert_eq!(
            super::rss_bytes("Name:\ttest\nVmRSS:\t1234 kB\n"),
            Some(1_263_616)
        );
        assert_eq!(super::rss_bytes("Name:\ttest\n"), None);
    }
}
