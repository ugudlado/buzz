use crate::{config, env, identity::Identity, wire::DeployRequest};
use rand::RngExt;
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

const UNIT_MARKER: &str = "# Managed by buzz-backend-host";

pub fn deploy(request: &DeployRequest) -> Result<String, String> {
    let identity = Identity::from_nsec(&request.agent.private_key_nsec)?;
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| "remote HOME is not an absolute path".to_string())?;
    let unit_name = identity.service_name();
    let unit_path = home.join(".config/systemd/user").join(&unit_name);

    match systemctl(["is-active", "--quiet", &unit_name])?.code() {
        Some(0) => {
            verify_owned_unit(&unit_path, identity.pubkey())?;
            return Ok(unit_name);
        }
        Some(3 | 4) => {}
        _ => return Err("could not query the systemd user service manager".into()),
    }
    if unit_path.exists() {
        verify_owned_unit(&unit_path, identity.pubkey())?;
    }
    let config = config::parse(&request.provider_config)?;
    let workspace = resolve_existing_dir(config.workspace_dir.as_deref(), &home, "workspace_dir")?;
    let repos = resolve_repos_dir(config.repos_dir.as_deref(), &workspace, &home)?;

    let launch = request
        .agent
        .launch
        .as_ref()
        .ok_or_else(|| "deploy refused: launch block is required".to_string())?;
    let requested_agent = launch
        .command
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "deploy refused: launch.command is required".to_string())?;
    let remote_path = augmented_path(&home)?;
    let acp = resolve_executable("buzz-acp", &remote_path)?;
    let agent = resolve_executable(requested_agent, &remote_path)?;
    let mcp = resolve_executable("buzz-dev-mcp", &remote_path)?;
    resolve_executable("buzz", &remote_path)?;
    let git = resolve_executable("git", &remote_path)?;
    validate_git_version(&git)?;
    let git_credential = resolve_executable("git-credential-nostr", &remote_path)?;

    let generation = format!("{:08x}", rand::rng().random::<u32>());
    let resolved_env = env::build(
        &request.agent,
        env::RemoteInputs {
            generation: &generation,
            agent_command: path_text(&agent)?,
            mcp_command: path_text(&mcp)?,
            home: path_text(&home)?,
            path: &remote_path,
            repos_dir: path_text(&repos)?,
            git_command: path_text(&git)?,
            git_credential_helper: path_text(&git_credential)?,
        },
    )?;
    let state_dir = home
        .join(".local/state/buzz/agents")
        .join(identity.state_name());
    create_private_dir(&state_dir)?;
    let generation_path = state_dir.join(format!("{generation}.json"));
    write_atomic(
        &generation_path,
        &serde_json::to_vec(&resolved_env).map_err(secret_free)?,
        0o600,
    )?;

    let executable = match std::env::current_exe() {
        Ok(executable) => executable,
        Err(error) => {
            remove_staged_generation(&generation_path);
            return Err(format!("could not resolve remote helper path: {error}"));
        }
    };
    let unit = match unit_file(
        identity.pubkey(),
        &executable,
        &generation_path,
        &acp,
        &workspace,
    ) {
        Ok(unit) => unit,
        Err(error) => {
            remove_staged_generation(&generation_path);
            return Err(error);
        }
    };
    if let Some(parent) = unit_path.parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            remove_staged_generation(&generation_path);
            return Err(format!("could not create systemd directory: {error}"));
        }
    }
    if let Err(error) = write_atomic(&unit_path, unit.as_bytes(), 0o600) {
        remove_staged_generation(&generation_path);
        return Err(error);
    }

    ensure_success(systemctl(["daemon-reload"]), "systemd daemon-reload")?;
    ensure_success(systemctl(["start", &unit_name]), "start agent service")?;
    if systemctl(["is-active", "--quiet", &unit_name])?.code() != Some(0) {
        return Err(format!("{unit_name} did not remain active after start"));
    }
    if let Err(error) = cleanup_old_generations(&state_dir, &generation_path) {
        eprintln!("buzz-backend-host: {error}");
    }
    Ok(unit_name)
}

pub fn run(generation_path: &Path, acp: &Path, workspace: &Path) -> Result<(), String> {
    let bytes =
        fs::read(generation_path).map_err(|e| format!("could not read agent generation: {e}"))?;
    let env: BTreeMap<String, String> =
        serde_json::from_slice(&bytes).map_err(|_| "agent generation is invalid".to_string())?;
    let mut command = Command::new(acp);
    command.envs(env).current_dir(workspace);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        Err(format!("could not exec buzz-acp: {}", command.exec()))
    }
    #[cfg(not(unix))]
    {
        let _ = command;
        Err("the remote runner requires Unix".into())
    }
}

fn augmented_path(home: &Path) -> Result<String, String> {
    let mut paths = vec![home.join(".local/bin")];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    std::env::join_paths(paths)
        .map_err(|_| "remote PATH contains an unsupported path".to_string())?
        .into_string()
        .map_err(|_| "remote PATH is not UTF-8".to_string())
}

fn expand_remote_path(value: &str, home: &Path) -> PathBuf {
    if value == "~" {
        home.to_path_buf()
    } else if let Some(relative) = value.strip_prefix("~/") {
        home.join(relative)
    } else {
        PathBuf::from(value)
    }
}

fn canonical_dir(path: &Path, field: &str) -> Result<PathBuf, String> {
    let metadata = fs::metadata(path)
        .map_err(|e| format!("provider_config.{field} is not accessible: {e}"))?;
    if !metadata.is_dir() {
        return Err(format!("provider_config.{field} must be a directory"));
    }
    path.canonicalize()
        .map_err(|e| format!("provider_config.{field} could not be canonicalized: {e}"))
}

fn resolve_existing_dir(
    configured: Option<&str>,
    home: &Path,
    field: &str,
) -> Result<PathBuf, String> {
    canonical_dir(
        &configured
            .map(|value| expand_remote_path(value, home))
            .unwrap_or_else(|| home.to_path_buf()),
        field,
    )
}

fn resolve_repos_dir(
    configured: Option<&str>,
    workspace: &Path,
    home: &Path,
) -> Result<PathBuf, String> {
    if let Some(value) = configured {
        return canonical_dir(&expand_remote_path(value, home), "repos_dir");
    }
    let path = workspace.join("REPOS");
    if !path.exists() {
        create_private_dir(&path)
            .map_err(|e| format!("could not create default repositories folder: {e}"))?;
    }
    canonical_dir(&path, "repos_dir")
}

fn resolve_executable(command: &str, search_path: &str) -> Result<PathBuf, String> {
    let path = Path::new(command);
    let candidates: Vec<PathBuf> = if path.components().count() > 1 {
        vec![path.to_path_buf()]
    } else {
        std::env::split_paths(search_path)
            .map(|dir| dir.join(path))
            .collect()
    };
    candidates
        .into_iter()
        .find(|candidate| is_executable(candidate))
        .and_then(|candidate| candidate.canonicalize().ok())
        .ok_or_else(|| format!("required remote command {command:?} was not found on PATH"))
}

fn validate_git_version(git: &Path) -> Result<(), String> {
    let output = Command::new(git)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("could not query remote Git version: {e}"))?;
    if !output.status.success() {
        return Err("remote Git --version failed".into());
    }
    let version = std::str::from_utf8(&output.stdout)
        .ok()
        .and_then(parse_git_version)
        .ok_or_else(|| "remote Git returned an unrecognized version".to_string())?;
    if !supported_git_version(version) {
        return Err(format!(
            "remote Git {}.{}.{} is unsupported; Buzz requires Git 2.46 or newer",
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

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        metadata.is_file()
    }
}

fn systemctl<const N: usize>(args: [&str; N]) -> Result<std::process::ExitStatus, String> {
    Command::new("systemctl")
        .arg("--user")
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|e| format!("could not run systemctl --user: {e}"))
}

fn ensure_success(
    status: Result<std::process::ExitStatus, String>,
    action: &str,
) -> Result<(), String> {
    if status?.success() {
        Ok(())
    } else {
        Err(format!("could not {action}"))
    }
}

fn verify_owned_unit(path: &Path, pubkey: &str) -> Result<(), String> {
    let content = fs::read_to_string(path).map_err(|_| {
        "refusing to manage an active service without a Buzz-owned unit".to_string()
    })?;
    let marker = format!("{UNIT_MARKER}\n# Agent-Pubkey: {pubkey}\n");
    if content.starts_with(&marker) {
        Ok(())
    } else {
        Err("refusing to replace a systemd unit not owned by this Buzz agent".into())
    }
}

fn unit_file(
    pubkey: &str,
    runner: &Path,
    generation: &Path,
    acp: &Path,
    workspace: &Path,
) -> Result<String, String> {
    Ok(format!(
        "{UNIT_MARKER}\n# Agent-Pubkey: {pubkey}\n[Unit]\nDescription=Buzz managed agent\n\n[Service]\nType=exec\nExecStart={} run {} {} {}\nWorkingDirectory={}\nRestart=no\nKillMode=control-group\nTimeoutStopSec=240\n",
        systemd_arg(path_text(runner)?)?,
        systemd_arg(path_text(generation)?)?,
        systemd_arg(path_text(acp)?)?,
        systemd_arg(path_text(workspace)?)?,
        systemd_arg(path_text(workspace)?)?,
    ))
}

fn systemd_arg(value: &str) -> Result<String, String> {
    if value.contains(['\n', '\r', '\0']) {
        return Err("systemd path contains a control character".into());
    }
    Ok(format!(
        "\"{}\"",
        value
            .replace('%', "%%")
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
    ))
}

fn path_text(path: &Path) -> Result<&str, String> {
    path.to_str()
        .ok_or_else(|| "remote executable path is not UTF-8".to_string())
}

fn create_private_dir(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|e| format!("could not create agent state directory: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("could not protect agent state directory: {e}"))?;
    }
    Ok(())
}

fn write_atomic(path: &Path, bytes: &[u8], _mode: u32) -> Result<(), String> {
    let mut temp = path.to_path_buf();
    temp.set_extension(format!("tmp-{}", std::process::id()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(_mode);
    }
    let mut file = options
        .open(&temp)
        .map_err(|e| format!("could not stage managed agent file: {e}"))?;
    if let Err(error) = file.write_all(bytes).and_then(|_| file.sync_all()) {
        let _ = fs::remove_file(&temp);
        return Err(format!("could not write managed agent file: {error}"));
    }
    fs::rename(&temp, path).map_err(|e| {
        let _ = fs::remove_file(&temp);
        format!("could not publish managed agent file: {e}")
    })
}

fn remove_staged_generation(path: &Path) {
    let _ = fs::remove_file(path);
}

fn cleanup_old_generations(state_dir: &Path, current: &Path) -> Result<(), String> {
    let entries = fs::read_dir(state_dir)
        .map_err(|e| format!("agent started, but old launch files could not be inspected: {e}"))?;
    let mut warning = None;
    for entry in entries {
        let path = match entry {
            Ok(entry) => entry.path(),
            Err(error) => {
                warning.get_or_insert_with(|| {
                    format!("agent started, but an old launch file could not be inspected: {error}")
                });
                continue;
            }
        };
        if path != current && is_generation_file(&path) {
            if let Err(error) = fs::remove_file(&path) {
                warning.get_or_insert_with(|| {
                    format!("agent started, but an old launch file could not be removed: {error}")
                });
            }
        }
    }
    warning.map_or(Ok(()), Err)
}

fn is_generation_file(path: &Path) -> bool {
    path.extension().and_then(|value| value.to_str()) == Some("json")
        && path
            .file_stem()
            .and_then(|value| value.to_str())
            .is_some_and(|stem| {
                stem.len() == 8
                    && stem
                        .chars()
                        .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
            })
}

fn secret_free(_: serde_json::Error) -> String {
    "could not encode agent environment".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_has_no_secrets_and_never_restarts() {
        let unit = unit_file(
            &"a".repeat(64),
            Path::new("/home/agent/.local/bin/buzz-backend-host"),
            Path::new("/home/agent/.local/state/buzz/generation.json"),
            Path::new("/home/agent/.local/bin/buzz-acp"),
            Path::new("/srv/buzz workspace"),
        )
        .unwrap();
        assert!(unit.contains("Restart=no"));
        assert!(unit.contains("Type=exec"));
        assert!(unit.contains("KillMode=control-group"));
        assert!(unit.contains("WorkingDirectory=\"/srv/buzz workspace\""));
        assert!(!unit.contains("BUZZ_PRIVATE_KEY"));
        assert!(!unit.contains("nsec1"));
    }

    #[test]
    fn systemd_paths_are_quoted_and_specifiers_escaped() {
        assert_eq!(systemd_arg("/tmp/a b%/c").unwrap(), "\"/tmp/a b%%/c\"");
        assert!(systemd_arg("/tmp/a\nb").is_err());
    }

    #[test]
    fn resolves_existing_configured_dirs_and_only_creates_the_default_repos_dir() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/host-provider-path-tests")
            .join(format!(
                "{}-{:08x}",
                std::process::id(),
                rand::rng().random::<u32>()
            ));
        let workspace = root.join("workspace");
        let repos = root.join("repositories");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&repos).unwrap();

        assert_eq!(
            resolve_existing_dir(Some("~/workspace"), &root, "workspace_dir").unwrap(),
            workspace.canonicalize().unwrap()
        );
        assert_eq!(
            resolve_repos_dir(Some("~/repositories"), &workspace, &root).unwrap(),
            repos.canonicalize().unwrap()
        );
        assert!(resolve_repos_dir(Some("~/missing"), &workspace, &root).is_err());
        assert!(!root.join("missing").exists());

        let default_repos = resolve_repos_dir(None, &workspace, &root).unwrap();
        assert_eq!(
            default_repos,
            workspace.join("REPOS").canonicalize().unwrap()
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(default_repos).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn parses_and_gates_git_versions() {
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
}
