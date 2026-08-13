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
use std::process::Stdio;
use std::time::Duration;

use url::Url;

const GIT_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AutoCloneKind {
    Relay,
    PublicGithub,
}

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

fn clone_destination_name(repo_dtag: &str, clone_url: &str) -> Option<String> {
    local_repo_candidates(repo_dtag, Some(clone_url))
        .into_iter()
        .next()
}

fn validate_auto_clone_url(
    clone_url: &str,
    relay_http_base: &str,
) -> Result<AutoCloneKind, String> {
    let clone = Url::parse(clone_url).map_err(|_| "linked repository clone URL is invalid")?;
    if !clone.username().is_empty()
        || clone.password().is_some()
        || clone.query().is_some()
        || clone.fragment().is_some()
    {
        return Err(
            "linked repository clone URL must not contain credentials, query, or fragment".into(),
        );
    }

    let segments = clone
        .path_segments()
        .map(|parts| parts.filter(|part| !part.is_empty()).collect::<Vec<_>>())
        .unwrap_or_default();
    let valid_github_part = |part: &&str| {
        !part.starts_with('-')
            && !part.contains("..")
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    };
    if clone.scheme() == "https"
        && clone.host_str() == Some("github.com")
        && clone.port().is_none()
        && segments.len() == 2
        && segments.iter().all(valid_github_part)
    {
        return Ok(AutoCloneKind::PublicGithub);
    }

    let relay = Url::parse(relay_http_base)
        .map_err(|_| "configured relay URL is invalid for repository access")?;
    if !matches!(clone.scheme(), "http" | "https")
        || clone.scheme() != relay.scheme()
        || clone.host_str() != relay.host_str()
        || clone.port_or_known_default() != relay.port_or_known_default()
    {
        return Err(
            "automatic repository access is limited to the active Buzz relay or public GitHub"
                .into(),
        );
    }
    let relay_path = relay.path().trim_end_matches('/');
    let relative = clone
        .path()
        .strip_prefix(relay_path)
        .and_then(|path| path.strip_prefix('/'))
        .unwrap_or_default();
    let parts = relative
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let valid_repo_id = |value: &str| {
        !value.starts_with('-')
            && !value.contains("..")
            && value
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    };
    let valid = parts.len() == 3
        && parts[0] == "git"
        && parts[1].len() == 64
        && parts[1].chars().all(|c| c.is_ascii_hexdigit())
        && valid_repo_id(parts[2]);
    if !valid {
        return Err("linked repository URL is not a repository on the active Buzz relay".into());
    }
    Ok(AutoCloneKind::Relay)
}

fn resolve_command(env_key: &str, default_command: &str) -> Option<PathBuf> {
    let configured = std::env::var_os(env_key).filter(|value| !value.is_empty());
    if let Some(configured) = configured {
        return PathBuf::from(configured).canonicalize().ok();
    }
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .map(|directory| directory.join(default_command))
        .find(|candidate| candidate.is_file())
        .and_then(|candidate| candidate.canonicalize().ok())
}

async fn validate_git_version(git: &Path) -> Result<(), String> {
    let mut command = tokio::process::Command::new(git);
    command
        .kill_on_drop(true)
        .arg("--version")
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    let output = tokio::time::timeout(Duration::from_secs(10), command.output())
        .await
        .map_err(|_| "Git version check timed out".to_string())?
        .map_err(|error| format!("could not query Git version: {error}"))?;
    if !output.status.success() {
        return Err("Git --version failed".into());
    }
    let version = std::str::from_utf8(&output.stdout)
        .ok()
        .and_then(parse_git_version)
        .ok_or_else(|| "Git returned an unrecognized version".to_string())?;
    if !supported_git_version(version) {
        return Err(format!(
            "Git {}.{}.{} is unsupported; Buzz requires Git 2.46 or newer",
            version.0, version.1, version.2
        ));
    }
    Ok(())
}

fn supported_git_version(version: (u64, u64, u64)) -> bool {
    version >= (2, 46, 0)
}

fn parse_git_version(value: &str) -> Option<(u64, u64, u64)> {
    let version = value
        .trim()
        .strip_prefix("git version ")?
        .split_whitespace()
        .next()?;
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    Some((major, minor, patch))
}

fn hardened_git_command(
    git: &Path,
    relay_http_base: &str,
    kind: AutoCloneKind,
) -> Result<tokio::process::Command, String> {
    let mut command = tokio::process::Command::new(git);
    command
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null");
    for key in [
        "GIT_DIR",
        "GIT_COMMON_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_CONFIG",
        "GIT_CONFIG_PARAMETERS",
        "GIT_CEILING_DIRECTORIES",
        "GIT_DISCOVERY_ACROSS_FILESYSTEM",
        "GIT_EXEC_PATH",
        "GIT_SSH_COMMAND",
        "GIT_PROXY_COMMAND",
        "GIT_ASKPASS",
        "SSH_ASKPASS",
        "GIT_EXTERNAL_DIFF",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ] {
        command.env_remove(key);
    }
    let mut entries = vec![
        ("credential.helper".to_string(), String::new()),
        ("core.hooksPath".to_string(), "/dev/null".to_string()),
        ("core.fsmonitor".to_string(), "false".to_string()),
        ("protocol.allow".to_string(), "never".to_string()),
        ("protocol.http.allow".to_string(), "always".to_string()),
        ("protocol.https.allow".to_string(), "always".to_string()),
        ("protocol.file.allow".to_string(), "never".to_string()),
        ("protocol.ext.allow".to_string(), "never".to_string()),
        ("http.followRedirects".to_string(), "false".to_string()),
    ];
    if kind == AutoCloneKind::Relay {
        let helper = resolve_command("BUZZ_ACP_GIT_CREDENTIAL_HELPER", "git-credential-nostr")
            .ok_or_else(|| "git-credential-nostr was not found on PATH".to_string())?;
        entries.push((
            format!(
                "credential.{}/git.helper",
                relay_http_base.trim_end_matches('/')
            ),
            helper.to_string_lossy().replace('\\', "/"),
        ));
        entries.push((
            format!(
                "credential.{}/git.useHttpPath",
                relay_http_base.trim_end_matches('/')
            ),
            "true".to_string(),
        ));
    }
    command.env("GIT_CONFIG_COUNT", entries.len().to_string());
    for (index, (key, value)) in entries.into_iter().enumerate() {
        command.env(format!("GIT_CONFIG_KEY_{index}"), key);
        command.env(format!("GIT_CONFIG_VALUE_{index}"), value);
    }
    Ok(command)
}

async fn run_git(mut command: tokio::process::Command, action: &str) -> Result<(), String> {
    let status = tokio::time::timeout(GIT_TIMEOUT, command.status())
        .await
        .map_err(|_| format!("repository {action} timed out"))?
        .map_err(|error| format!("could not start git {action}: {error}"))?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| format!("git {action} failed with status {status}"))
}

async fn reject_repo_transport_overrides(
    git: &Path,
    relay_http_base: &str,
    kind: AutoCloneKind,
    repo_dir: &Path,
    approved_url: &str,
) -> Result<(), String> {
    let mut command = hardened_git_command(git, relay_http_base, kind)?;
    command
        .stdout(Stdio::piped())
        .arg("-C")
        .arg(repo_dir)
        .args([
            "config",
            "--local",
            "--includes",
            "--get-regexp",
            r"^(url\..*\.insteadof|http(\..*)?\.(proxy|curloptresolve|followredirects)|remote\.origin\.proxy)$",
        ]);
    let output = tokio::time::timeout(GIT_TIMEOUT, command.output())
        .await
        .map_err(|_| "repository URL rewrite inspection timed out".to_string())?
        .map_err(|error| format!("could not inspect repository URL rewrites: {error}"))?;
    match output.status.code() {
        Some(1) if output.stdout.is_empty() => {}
        Some(0) => {
            return Err("repository fetch refused because local Git transport overrides could redirect its approved origin".into());
        }
        _ => return Err("could not safely inspect repository URL rewrite rules".into()),
    }

    let mut command = hardened_git_command(git, relay_http_base, kind)?;
    command
        .stdout(Stdio::piped())
        .arg("-C")
        .arg(repo_dir)
        .args([
            "config",
            "--local",
            "--includes",
            "--get-all",
            "remote.origin.url",
        ]);
    let output = tokio::time::timeout(GIT_TIMEOUT, command.output())
        .await
        .map_err(|_| "repository origin inspection timed out".to_string())?
        .map_err(|error| format!("could not inspect repository origin: {error}"))?;
    if !output.status.success() {
        return Err("could not safely inspect repository origin".into());
    }
    let urls = String::from_utf8(output.stdout)
        .map_err(|_| "repository origin URL is not UTF-8".to_string())?;
    let urls = urls.lines().collect::<Vec<_>>();
    if urls.len() != 1 || normalized_clone_url(urls[0]) != normalized_clone_url(approved_url) {
        return Err(
            "repository fetch refused because its effective origin is not exactly the approved URL"
                .into(),
        );
    }
    Ok(())
}

async fn fetch_origin(mut command: tokio::process::Command, repo_dir: &Path) -> Result<(), String> {
    command
        .arg("-C")
        .arg(repo_dir)
        .args(["fetch", "--no-recurse-submodules", "origin"]);
    run_git(command, "fetch").await
}

async fn clone_into(
    mut command: tokio::process::Command,
    clone_url: &str,
    temporary: &Path,
) -> Result<(), String> {
    command.args(["clone", "--no-recurse-submodules", "--end-of-options"]);
    command.arg(clone_url).arg(temporary);
    run_git(command, "clone").await
}

fn finalize_clone(temporary: &Path, destination: &Path) -> Result<PathBuf, String> {
    match std::fs::rename(temporary, destination) {
        Ok(()) => destination
            .canonicalize()
            .map_err(|error| format!("could not resolve cloned repository: {error}")),
        Err(error) if destination.exists() => Err(format!(
            "repository destination {} was created concurrently; refusing to replace it ({error})",
            destination.display()
        )),
        Err(error) => Err(format!("could not finalize repository clone: {error}")),
    }
}

struct CloneCleanup(PathBuf);

impl Drop for CloneCleanup {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0) {
            if error.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(path = %self.0.display(), "could not remove failed clone: {error}");
            }
        }
    }
}

/// Prepare the linked repository for a channel session. Existing checkouts with
/// an allowed automatic URL are fetched without touching their worktree; other
/// existing origins remain usable but are never contacted automatically.
pub(crate) async fn prepare_repo_dir(
    repos_root: &Path,
    repo_dtag: &str,
    clone_url: Option<&str>,
    relay_http_base: &str,
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(repos_root)
        .map_err(|error| format!("could not create repositories directory: {error}"))?;
    let repos_root = repos_root
        .canonicalize()
        .map_err(|error| format!("could not resolve repositories directory: {error}"))?;

    if let Some(existing) = find_local_repo_dir(&repos_root, repo_dtag, clone_url) {
        let Some(clone_url) = clone_url else {
            return Ok(existing);
        };
        let Ok(kind) = validate_auto_clone_url(clone_url, relay_http_base) else {
            return Ok(existing);
        };
        let git = resolve_command("BUZZ_ACP_GIT_COMMAND", "git")
            .ok_or_else(|| "git was not found on PATH".to_string())?;
        validate_git_version(&git).await?;
        reject_repo_transport_overrides(&git, relay_http_base, kind, &existing, clone_url).await?;
        let command = hardened_git_command(&git, relay_http_base, kind)?;
        fetch_origin(command, &existing).await?;
        return Ok(existing);
    }

    let clone_url = clone_url.ok_or_else(|| {
        "linked repository has no clone URL and no matching local checkout".to_string()
    })?;
    let kind = validate_auto_clone_url(clone_url, relay_http_base)?;
    let destination_name = clone_destination_name(repo_dtag, clone_url)
        .ok_or_else(|| "could not derive a safe repository directory name".to_string())?;
    let destination = repos_root.join(destination_name);
    if destination.exists() {
        return Err(format!(
            "repository destination {} already exists with a different origin",
            destination.display()
        ));
    }

    let git = resolve_command("BUZZ_ACP_GIT_COMMAND", "git")
        .ok_or_else(|| "git was not found on PATH".to_string())?;
    validate_git_version(&git).await?;
    let temporary = repos_root.join(format!(".buzz-clone-{}", uuid::Uuid::new_v4()));
    let cleanup = CloneCleanup(temporary.clone());
    let command = hardened_git_command(&git, relay_http_base, kind)?;
    clone_into(command, clone_url, &temporary).await?;
    let final_path = finalize_clone(&temporary, &destination)?;
    drop(cleanup);
    Ok(final_path)
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

    #[test]
    fn automatic_urls_are_limited_to_active_relay_and_public_github() {
        let owner = "a".repeat(64);
        assert_eq!(
            validate_auto_clone_url(
                &format!("https://relay.example/prefix/git/{owner}/repo"),
                "https://relay.example/prefix"
            ),
            Ok(AutoCloneKind::Relay)
        );
        assert_eq!(
            validate_auto_clone_url(
                "https://github.com/block/buzz.git",
                "https://relay.example/prefix"
            ),
            Ok(AutoCloneKind::PublicGithub)
        );
        for rejected in [
            format!("ssh://relay.example/prefix/git/{owner}/repo"),
            format!("file:///tmp/git/{owner}/repo"),
            format!("ext::sh -c bad/git/{owner}/repo"),
            format!("https://other.example/prefix/git/{owner}/repo"),
            format!("https://relay.example/other/git/{owner}/repo"),
            format!("https://relay.example/prefix/git/{owner}/repo%2Fescape"),
            "https://github.com/block/buzz/issues".to_string(),
            "https://user@github.com/block/buzz".to_string(),
        ] {
            assert!(
                validate_auto_clone_url(&rejected, "https://relay.example/prefix").is_err(),
                "accepted {rejected}"
            );
        }
    }

    #[test]
    fn parses_and_gates_minimum_git_version() {
        assert_eq!(parse_git_version("git version 2.46.0\n"), Some((2, 46, 0)));
        assert_eq!(
            parse_git_version("git version 2.49.0 (Apple Git-154)"),
            Some((2, 49, 0))
        );
        assert_eq!(parse_git_version("git version 3.0"), Some((3, 0, 0)));
        assert_eq!(parse_git_version("not git"), None);
        assert!(!supported_git_version((2, 45, 9)));
        assert!(supported_git_version((2, 46, 0)));
        assert!(supported_git_version((3, 0, 0)));
    }

    #[tokio::test]
    async fn existing_checkout_with_unapproved_origin_is_used_without_fetch() {
        let tmp = tempfile_dir();
        let repos_root = tmp.join("REPOS");
        let repo_dir = repos_root.join("someone--repo");
        std::fs::create_dir_all(repo_dir.join(".git")).unwrap();
        std::fs::write(
            repo_dir.join(".git/config"),
            "[remote \"origin\"]\n\turl = https://git.example/someone/repo\n",
        )
        .unwrap();

        let found = prepare_repo_dir(
            &repos_root,
            "repo",
            Some("https://git.example/someone/repo"),
            "https://relay.example",
        )
        .await
        .unwrap();
        assert_eq!(found, repo_dir.canonicalize().unwrap());
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[tokio::test]
    async fn local_transport_overrides_block_approved_fetch() {
        let tmp = tempfile_dir();
        let repo = tmp.join("repo");
        git(&["init", repo.to_str().unwrap()], None);
        git(
            &[
                "config",
                "url.file:///attacker/.insteadOf",
                "https://github.com/",
            ],
            Some(&repo),
        );
        for (key, value) in [
            ("url.file:///attacker/.insteadOf", "https://github.com/"),
            ("http.proxy", "http://attacker.invalid"),
            (
                "http.https://github.com/.curloptResolve",
                "github.com:443:127.0.0.1",
            ),
            ("http.followRedirects", "true"),
            ("remote.origin.proxy", "attacker-proxy"),
        ] {
            git(&["config", key, value], Some(&repo));
            let error = reject_repo_transport_overrides(
                Path::new("git"),
                "https://relay.example",
                AutoCloneKind::PublicGithub,
                &repo,
                "https://github.com/block/buzz",
            )
            .await
            .unwrap_err();
            assert!(error.contains("transport overrides"), "{key}: {error}");
            git(&["config", "--unset-all", key], Some(&repo));
        }
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[tokio::test]
    async fn included_origin_cannot_precede_the_approved_origin() {
        let tmp = tempfile_dir();
        let repo = tmp.join("repo");
        git(&["init", repo.to_str().unwrap()], None);
        let included = tmp.join("attacker-config");
        std::fs::write(
            &included,
            "[remote \"origin\"]\n\turl = https://github.com/attacker/repo\n",
        )
        .unwrap();
        std::fs::write(
            repo.join(".git/config"),
            format!(
                "[include]\n\tpath = {}\n[remote \"origin\"]\n\turl = https://github.com/block/buzz\n",
                included.display()
            ),
        )
        .unwrap();

        let error = reject_repo_transport_overrides(
            Path::new("git"),
            "https://relay.example",
            AutoCloneKind::PublicGithub,
            &repo,
            "https://github.com/block/buzz",
        )
        .await
        .unwrap_err();
        assert!(error.contains("effective origin"), "{error}");
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn hardened_git_clears_inherited_config_and_proxy_environment() {
        let command = hardened_git_command(
            Path::new("git"),
            "https://relay.example",
            AutoCloneKind::PublicGithub,
        )
        .unwrap();
        let env = command
            .as_std()
            .get_envs()
            .collect::<std::collections::HashMap<_, _>>();
        for key in [
            "GIT_CONFIG_PARAMETERS",
            "GIT_CONFIG",
            "GIT_EXEC_PATH",
            "GIT_PROXY_COMMAND",
            "GIT_ASKPASS",
            "SSH_ASKPASS",
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
        ] {
            assert_eq!(env.get(std::ffi::OsStr::new(key)), Some(&None), "{key}");
        }
        assert!(env
            .values()
            .flatten()
            .any(|value| *value == std::ffi::OsStr::new("false")));
    }

    #[tokio::test]
    async fn occupied_canonical_destination_fails_closed() {
        let tmp = tempfile_dir();
        let repos_root = tmp.join("REPOS");
        let owner = "a".repeat(64);
        let occupied = repos_root.join(format!("{owner}--repo"));
        std::fs::create_dir_all(&occupied).unwrap();
        let error = prepare_repo_dir(
            &repos_root,
            "repo",
            Some(&format!("https://relay.example/git/{owner}/repo")),
            "https://relay.example",
        )
        .await
        .unwrap_err();
        assert!(error.contains("different origin"), "{error}");
        assert!(occupied.exists());
        std::fs::remove_dir_all(&tmp).ok();
    }

    fn git(args: &[&str], cwd: Option<&Path>) -> String {
        let mut command = std::process::Command::new("git");
        command.args(args);
        if let Some(cwd) = cwd {
            command.current_dir(cwd);
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn seed_origin(root: &Path) -> (PathBuf, PathBuf) {
        let origin = root.join("origin.git");
        let seed = root.join("seed");
        git(&["init", "--bare", origin.to_str().unwrap()], None);
        git(&["init", seed.to_str().unwrap()], None);
        git(&["config", "user.name", "Buzz Test"], Some(&seed));
        git(
            &["config", "user.email", "buzz-test@example.invalid"],
            Some(&seed),
        );
        std::fs::write(seed.join("tracked.txt"), "one\n").unwrap();
        git(&["add", "tracked.txt"], Some(&seed));
        git(&["commit", "-m", "initial"], Some(&seed));
        git(&["branch", "-M", "main"], Some(&seed));
        git(
            &["remote", "add", "origin", origin.to_str().unwrap()],
            Some(&seed),
        );
        git(&["push", "-u", "origin", "main"], Some(&seed));
        git(
            &[
                "--git-dir",
                origin.to_str().unwrap(),
                "symbolic-ref",
                "HEAD",
                "refs/heads/main",
            ],
            None,
        );
        (origin, seed)
    }

    #[tokio::test]
    async fn real_git_clone_is_finalized_atomically() {
        let tmp = tempfile_dir();
        let (origin, _) = seed_origin(&tmp);
        let repos = tmp.join("REPOS");
        std::fs::create_dir_all(&repos).unwrap();
        let temporary = repos.join(".buzz-clone-test");
        let destination = repos.join("owner--repo");
        let cleanup = CloneCleanup(temporary.clone());
        let command = tokio::process::Command::new("git");
        clone_into(command, origin.to_str().unwrap(), &temporary)
            .await
            .unwrap();
        let final_path = finalize_clone(&temporary, &destination).unwrap();
        drop(cleanup);

        assert_eq!(final_path, destination.canonicalize().unwrap());
        assert!(!temporary.exists());
        assert_eq!(
            git(&["rev-parse", "HEAD"], Some(&destination)),
            git(&["rev-parse", "main"], Some(&destination))
        );
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[tokio::test]
    async fn real_git_fetch_preserves_head_index_worktree_and_untracked_files() {
        let tmp = tempfile_dir();
        let (origin, seed) = seed_origin(&tmp);
        let checkout = tmp.join("checkout");
        git(
            &[
                "clone",
                "--branch",
                "main",
                origin.to_str().unwrap(),
                checkout.to_str().unwrap(),
            ],
            None,
        );
        std::fs::write(checkout.join("staged.txt"), "staged\n").unwrap();
        git(&["add", "staged.txt"], Some(&checkout));
        std::fs::write(checkout.join("tracked.txt"), "local edit\n").unwrap();
        std::fs::write(checkout.join("untracked.txt"), "keep me\n").unwrap();
        let head_before = git(&["rev-parse", "HEAD"], Some(&checkout));
        let index_before = git(&["write-tree"], Some(&checkout));
        let status_before = git(&["status", "--porcelain=v1"], Some(&checkout));

        std::fs::write(seed.join("remote.txt"), "remote\n").unwrap();
        git(&["add", "remote.txt"], Some(&seed));
        git(&["commit", "-m", "remote update"], Some(&seed));
        git(&["push", "origin", "main"], Some(&seed));
        let remote_head = git(&["rev-parse", "HEAD"], Some(&seed));

        let command = tokio::process::Command::new("git");
        fetch_origin(command, &checkout).await.unwrap();

        assert_eq!(git(&["rev-parse", "HEAD"], Some(&checkout)), head_before);
        assert_eq!(git(&["write-tree"], Some(&checkout)), index_before);
        assert_eq!(
            git(&["status", "--porcelain=v1"], Some(&checkout)),
            status_before
        );
        assert_eq!(
            std::fs::read_to_string(checkout.join("untracked.txt")).unwrap(),
            "keep me\n"
        );
        assert_eq!(
            git(&["rev-parse", "origin/main"], Some(&checkout)),
            remote_head
        );
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[tokio::test]
    async fn failed_real_git_clone_removes_only_its_temporary_directory() {
        let tmp = tempfile_dir();
        let repos = tmp.join("REPOS");
        std::fs::create_dir_all(&repos).unwrap();
        let sentinel = repos.join("keep");
        std::fs::write(&sentinel, "safe").unwrap();
        let temporary = repos.join(".buzz-clone-test");
        {
            let _cleanup = CloneCleanup(temporary.clone());
            let command = tokio::process::Command::new("git");
            assert!(
                clone_into(command, "/definitely/missing/repository", &temporary)
                    .await
                    .is_err()
            );
        }
        assert!(!temporary.exists());
        assert_eq!(std::fs::read_to_string(sentinel).unwrap(), "safe");
        std::fs::remove_dir_all(&tmp).ok();
    }

    fn tempfile_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "buzz-acp-repo-paths-test-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
