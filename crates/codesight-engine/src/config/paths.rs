//! Path helpers shared by `AppConfig` layout methods.

use std::path::PathBuf;

pub fn slug_dir(base: PathBuf, repo: &str) -> PathBuf {
    let sanitized = repo
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' => ch,
            _ => '-',
        })
        .collect::<String>()
        .trim_matches('-')
        .to_owned();
    let digest = blake3::hash(repo.as_bytes()).to_hex().to_string();
    base.join(format!("{sanitized}-{}", &digest[..12]))
}
