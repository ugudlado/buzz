//! Resolution of a channel's linked repo to its local clone path under the
//! nest's `REPOS` directory.
//!
//! Ported from `desktop/src-tauri/src/commands/project_repo_paths.rs`'s
//! `find_local_repo_dir`/`local_repo_candidates` (same matching logic,
//! unchanged) — buzz-acp is a standalone binary that doesn't depend on
//! `desktop/src-tauri`, so this is a small, dependency-light duplicate
//! rather than a shared crate extraction. Unlike the desktop version, this
//! module takes a single `repos_root` (buzz-acp always knows its own nest
//! root — see `PromptContext.cwd`/`REPOS` in `pool.rs`) rather than
//! discovering multiple candidate roots from user config.

use std::path::{Path, PathBuf};

use url::Url;

fn local_repo_name_candidate(value: &str) -> Option<String> {
    let trimmed = value.trim().trim_end_matches(".git");
    if trimmed.is_empty()
        || trimmed == "."
        || trimmed == ".."
        || trimmed.contains('/')
        || trimmed.contains('\\')
    {
        return None;
    }
    Some(trimmed.to_string())
}

fn clone_url_repo_name(clone_url: &str) -> Option<String> {
    let parsed = Url::parse(clone_url).ok()?;
    let last_segment = parsed.path_segments()?.rfind(|part| !part.is_empty())?;
    local_repo_name_candidate(last_segment)
}

fn clone_url_owner_repo_name(clone_url: &str) -> Option<String> {
    let parsed = Url::parse(clone_url).ok()?;
    let parts = parsed
        .path_segments()?
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let [.., owner, repo] = parts.as_slice() else {
        return None;
    };
    local_repo_name_candidate(&format!(
        "{}--{}",
        local_repo_name_candidate(owner)?,
        local_repo_name_candidate(repo)?
    ))
}

fn normalized_clone_url(value: &str) -> &str {
    value.trim().trim_end_matches('/').trim_end_matches(".git")
}

fn checkout_git_config(repo_dir: &Path, repos_root: &Path) -> Option<PathBuf> {
    let dot_git = repo_dir.join(".git");
    let git_dir = if dot_git.is_dir() {
        dot_git
    } else {
        let pointer = std::fs::read_to_string(dot_git).ok()?;
        let git_dir = PathBuf::from(pointer.trim().strip_prefix("gitdir:")?.trim());
        if git_dir.is_absolute() {
            git_dir
        } else {
            repo_dir.join(git_dir)
        }
    };
    let git_dir = git_dir.canonicalize().ok()?;
    if !git_dir.starts_with(repos_root) {
        return None;
    }
    let config = git_dir.join("config").canonicalize().ok()?;
    config.starts_with(repos_root).then_some(config)
}

fn checkout_origin_matches(repo_dir: &Path, repos_root: &Path, clone_url: &str) -> bool {
    let Some(config_path) = checkout_git_config(repo_dir, repos_root) else {
        return false;
    };
    let Ok(config) = std::fs::read_to_string(config_path) else {
        return false;
    };
    let mut in_origin = false;
    for line in config.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_origin = line == r#"[remote "origin"]"#;
            continue;
        }
        if in_origin {
            if let Some((key, value)) = line.split_once('=') {
                if key.trim() == "url" {
                    return normalized_clone_url(value) == normalized_clone_url(clone_url);
                }
            }
        }
    }
    false
}

fn local_repo_candidates(repo_dtag: &str, clone_url: Option<&str>) -> Vec<String> {
    let mut candidates = Vec::new();
    if let Some(candidate) = clone_url.and_then(clone_url_owner_repo_name) {
        candidates.push(candidate);
    }
    if let Some(candidate) = local_repo_name_candidate(repo_dtag) {
        if !candidates.iter().any(|existing| existing == &candidate) {
            candidates.push(candidate);
        }
    }
    if let Some(candidate) = clone_url.and_then(clone_url_repo_name) {
        if !candidates.iter().any(|existing| existing == &candidate) {
            candidates.push(candidate);
        }
    }
    candidates
}

/// Resolve a repo's local clone directory under `repos_root`, matching by
/// candidate directory name (owner--repo, repo d-tag, or bare repo name)
/// and, when a `.git` checkout is found, verifying its `origin` remote
/// matches `clone_url`. Returns `None` (not an error) when no local clone
/// exists — the caller falls back to its own default cwd in that case.
pub(crate) fn find_local_repo_dir(
    repos_root: &Path,
    repo_dtag: &str,
    clone_url: Option<&str>,
) -> Option<PathBuf> {
    let repos_root = repos_root.canonicalize().ok()?;
    for candidate in local_repo_candidates(repo_dtag, clone_url) {
        let candidate_path = repos_root.join(candidate);
        let Ok(candidate_path) = candidate_path.canonicalize() else {
            continue;
        };
        if !candidate_path.starts_with(&repos_root) || !candidate_path.is_dir() {
            continue;
        }
        if candidate_path.join(".git").exists()
            && clone_url
                .map(|url| checkout_origin_matches(&candidate_path, &repos_root, url))
                .unwrap_or(true)
        {
            return Some(candidate_path);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_local_repo_dir_matches_owner_repo_directory_by_origin() {
        let tmp = tempfile_dir();
        let repos_root = tmp.join("REPOS");
        let repo_dir = repos_root.join("ugudlado--backlog");
        std::fs::create_dir_all(repo_dir.join(".git")).unwrap();
        std::fs::write(
            repo_dir.join(".git").join("config"),
            "[remote \"origin\"]\n\turl = https://github.com/ugudlado/backlog\n",
        )
        .unwrap();

        let found = find_local_repo_dir(
            &repos_root,
            "test-project",
            Some("https://github.com/ugudlado/backlog"),
        );
        assert_eq!(found, Some(repo_dir.canonicalize().unwrap()));

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn find_local_repo_dir_none_when_no_local_clone_exists() {
        let tmp = tempfile_dir();
        let repos_root = tmp.join("REPOS");
        std::fs::create_dir_all(&repos_root).unwrap();

        let found = find_local_repo_dir(
            &repos_root,
            "nonexistent-project",
            Some("https://github.com/nobody/nothing"),
        );
        assert_eq!(found, None);

        std::fs::remove_dir_all(&tmp).ok();
    }

    fn tempfile_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "buzz-acp-repo-paths-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
