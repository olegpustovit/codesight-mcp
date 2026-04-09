use std::path::Path;

/// Known documentation extensions that should NOT be flagged even if the
/// filename contains "secret".
const DOC_EXTENSIONS: &[&str] = &[".md", ".rst", ".txt", ".adoc", ".html", ".htm", ".ipynb"];

/// File extensions that are always secret material.
const SECRET_EXTENSIONS: &[&str] = &[".pem", ".key", ".p12", ".pfx", ".jks", ".keystore"];

/// Exact basenames (case-insensitive) that are secret.
const SECRET_BASENAMES: &[&str] = &[
    ".env",
    ".htpasswd",
    ".netrc",
    ".npmrc",
    ".pypirc",
    ".secrets",
    "id_rsa",
    "id_rsa.pub",
    "id_ed25519",
    "id_ed25519.pub",
    "id_dsa",
    "id_ecdsa",
    "credentials.json",
];

/// Basename suffixes that intentionally model filename-based secret policy.
const SECRET_SUFFIXES: &[&str] = &[".env", ".secrets"];

/// Returns `true` if the file at `path` looks like a secret or credential file.
///
/// These filename-based heuristics are intentional policy: they are meant to
/// block known secret-bearing filenames before content is read.
pub fn is_secret_file(path: &str) -> bool {
    let basename = Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    let lower_basename = basename.to_lowercase();

    for extension in DOC_EXTENSIONS {
        if lower_basename.ends_with(extension) {
            return false;
        }
    }

    for name in SECRET_BASENAMES {
        if lower_basename == *name {
            return true;
        }
    }

    for extension in SECRET_EXTENSIONS {
        if lower_basename.ends_with(extension) {
            return true;
        }
    }

    if lower_basename.starts_with(".env") || lower_basename.ends_with(".env") {
        return true;
    }

    if lower_basename.contains("secret") {
        return true;
    }

    for suffix in SECRET_SUFFIXES {
        if lower_basename.ends_with(suffix) && !lower_basename.starts_with(".env") {
            return true;
        }
    }

    lower_basename == "credentials" || lower_basename.starts_with("credentials.")
}
