use std::{
    fs,
    path::{Path, PathBuf},
};

pub fn normalize_repo_id(root: &Path) -> String {
    discover_remote_repo(root).unwrap_or_else(|| {
        let name = root
            .file_name()
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty())
            .unwrap_or("repo");
        // Include a short hash of the canonical path to avoid collisions
        // when two folders share the same basename.
        let canonical = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let hash = blake3::hash(canonical.to_string_lossy().as_bytes())
            .to_hex()
            .to_string();
        format!("local/{name}-{}", &hash[..8])
    })
}

fn discover_remote_repo(root: &Path) -> Option<String> {
    let config_path = git_config_path(root)?;
    let config = fs::read_to_string(config_path).ok()?;
    let mut in_origin = false;

    for line in config.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_origin = trimmed == "[remote \"origin\"]";
            continue;
        }
        if !in_origin {
            continue;
        }
        if let Some(url) = trimmed.strip_prefix("url =") {
            return parse_remote(url.trim());
        }
    }

    None
}

fn git_config_path(root: &Path) -> Option<PathBuf> {
    let dot_git = root.join(".git");
    if dot_git.is_dir() {
        return Some(dot_git.join("config"));
    }
    if !dot_git.is_file() {
        return None;
    }

    let content = fs::read_to_string(dot_git).ok()?;
    let target = content.strip_prefix("gitdir:")?.trim();
    let git_dir = if Path::new(target).is_absolute() {
        PathBuf::from(target)
    } else {
        root.join(target)
    };
    Some(git_dir.join("config"))
}

fn parse_remote(url: &str) -> Option<String> {
    let trimmed = url.trim().trim_end_matches(".git").trim_end_matches('/');
    trimmed
        .strip_prefix("https://github.com/")
        .or_else(|| trimmed.strip_prefix("git@github.com:"))
        .or_else(|| trimmed.strip_prefix("ssh://git@github.com/"))
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use super::parse_remote;

    #[test]
    fn parses_github_remotes() {
        assert_eq!(
            parse_remote("https://github.com/tiangolo/fastapi.git"),
            Some("tiangolo/fastapi".to_string())
        );
        assert_eq!(
            parse_remote("git@github.com:pallets/flask.git"),
            Some("pallets/flask".to_string())
        );
    }
}
