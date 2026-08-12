#![cfg(unix)]

use nostr::nips::nip19::ToBech32;
use rand::RngExt;
use serde_json::json;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn executable(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn run_helper(home: &Path, path: &str, payload: &serde_json::Value) -> serde_json::Value {
    let mut child = Command::new(env!("CARGO_BIN_EXE_buzz-backend-host"))
        .arg("remote-deploy")
        .env("HOME", home)
        .env("PATH", path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    serde_json::to_writer(child.stdin.take().unwrap(), payload).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn local_provider_keeps_secrets_on_ssh_stdin_and_hardens_the_connection() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/host-provider-tests")
        .join(format!(
            "local-{}-{:08x}",
            std::process::id(),
            rand::rng().random::<u32>()
        ));
    let bin = root.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let args_file = root.join("args");
    let stdin_file = root.join("stdin");
    executable(
        &bin.join("ssh"),
        &format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\ncat > '{}'\nprintf '%s\\n' '{{\"ok\":true,\"agent_id\":\"buzz-agent-test.service\"}}'\n",
            args_file.display(),
            stdin_file.display()
        ),
    );
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let keys = nostr::Keys::generate();
    let nsec = keys.secret_key().to_bech32().unwrap();
    let request = json!({
        "op": "deploy",
        "agent": {
            "relay_url": "wss://relay.example",
            "private_key_nsec": nsec,
            "auth_tag": "owner-tag",
            "launch": {"command": "hermes-acp", "owner_pubkey": "beef"}
        },
        "provider_config": {"host": "buzz-vps"}
    });
    let mut child = Command::new(env!("CARGO_BIN_EXE_buzz-backend-host"))
        .env("PATH", path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    serde_json::to_writer(child.stdin.take().unwrap(), &request).unwrap();
    let output = child.wait_with_output().unwrap();
    let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["ok"], true, "{response}");

    let args = fs::read_to_string(args_file).unwrap();
    for expected in [
        "-T",
        "BatchMode=yes",
        "ClearAllForwardings=yes",
        "ForwardAgent=no",
        "buzz-vps",
        "exec \"$HOME/.local/bin/buzz-backend-host\" remote-deploy",
    ] {
        assert!(
            args.lines().any(|line| line == expected),
            "missing {expected:?}: {args}"
        );
    }
    assert!(!args.contains(&nsec));
    assert!(fs::read_to_string(stdin_file).unwrap().contains(&nsec));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn oversized_ssh_output_is_drained_and_rejected_without_deadlock() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/host-provider-tests")
        .join(format!(
            "oversize-{}-{:08x}",
            std::process::id(),
            rand::rng().random::<u32>()
        ));
    let bin = root.join("bin");
    fs::create_dir_all(&bin).unwrap();
    executable(&bin.join("ssh"), "#!/bin/sh\nhead -c 1100000 /dev/zero\n");
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let keys = nostr::Keys::generate();
    let request = json!({
        "op": "deploy",
        "agent": {
            "relay_url": "wss://relay.example",
            "private_key_nsec": keys.secret_key().to_bech32().unwrap(),
            "auth_tag": "owner-tag",
            "launch": {"command": "hermes-acp", "owner_pubkey": "beef"}
        },
        "provider_config": {"host": "buzz-vps"}
    });
    let started = std::time::Instant::now();
    let mut child = Command::new(env!("CARGO_BIN_EXE_buzz-backend-host"))
        .env("PATH", path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    serde_json::to_writer(child.stdin.take().unwrap(), &request).unwrap();
    let output = child.wait_with_output().unwrap();
    let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["ok"], false, "{response}");
    assert!(response["error"]
        .as_str()
        .unwrap()
        .contains("exceeded the provider limit"));
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn remote_helper_writes_private_state_and_active_deploy_is_a_noop() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/host-provider-tests")
        .join(format!(
            "{}-{:08x}",
            std::process::id(),
            rand::rng().random::<u32>()
        ));
    let bin = root.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let active = root.join("active");
    let log = root.join("systemctl.log");
    executable(
        &bin.join("systemctl"),
        &format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\ncase \"$2\" in\n  is-active) test -f '{}' || exit 3;;\n  start) : > '{}';;\n  *) exit 0;;\nesac\n",
            log.display(),
            active.display(),
            active.display()
        ),
    );
    for command in ["buzz-acp", "hermes-acp", "buzz-dev-mcp"] {
        executable(&bin.join(command), "#!/bin/sh\nexit 0\n");
    }

    let keys = nostr::Keys::generate();
    let nsec = keys.secret_key().to_bech32().unwrap();
    let payload = json!({
        "agent": {
            "relay_url": "wss://relay.example",
            "private_key_nsec": nsec,
            "auth_tag": "owner-tag",
            "launch": {"command": "hermes-acp", "owner_pubkey": "beef"}
        },
        "provider_config": {"host": "ignored-remotely"}
    });
    let response = run_helper(&root, bin.to_str().unwrap(), &payload);
    assert_eq!(response["ok"], true, "{response}");

    let prefix = format!("buzz-agent-{}", &keys.public_key().to_hex()[..12]);
    let state_dir = root.join(".local/state/buzz/agents").join(&prefix);
    let generations: Vec<_> = fs::read_dir(&state_dir).unwrap().collect();
    assert_eq!(generations.len(), 1);
    let generation = generations[0].as_ref().unwrap().path();
    assert_eq!(fs::metadata(&generation).unwrap().mode() & 0o777, 0o600);
    let generation_text = fs::read_to_string(&generation).unwrap();
    assert!(generation_text.contains(&nsec));
    assert!(generation_text.contains(
        bin.join("hermes-acp")
            .canonicalize()
            .unwrap()
            .to_str()
            .unwrap()
    ));

    let unit = fs::read_to_string(
        root.join(".config/systemd/user")
            .join(format!("{prefix}.service")),
    )
    .unwrap();
    assert!(unit.contains("Restart=no"));
    assert!(!unit.contains(&nsec));

    let second = run_helper(&root, bin.to_str().unwrap(), &payload);
    assert_eq!(second["ok"], true, "{second}");
    assert_eq!(fs::read_dir(&state_dir).unwrap().count(), 1);
    assert_eq!(fs::read_to_string(&log).unwrap().lines().count(), 5);

    // A stopped service gets a fresh generation; the replaced private launch
    // file is removed only after systemd confirms the new service is active.
    fs::write(
        state_dir.join("notes.json"),
        "not provider generation state",
    )
    .unwrap();
    // A malformed provider-shaped entry makes GC warn, but must not turn a
    // systemd-confirmed start into a failed deploy response.
    fs::create_dir(state_dir.join("deadbeef.json")).unwrap();
    fs::remove_file(&active).unwrap();
    let third = run_helper(&root, bin.to_str().unwrap(), &payload);
    assert_eq!(third["ok"], true, "{third}");
    let remaining_generations = fs::read_dir(&state_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.is_file()
                && path.extension().and_then(|value| value.to_str()) == Some("json")
                && path.file_stem().unwrap().to_string_lossy().len() == 8
        })
        .count();
    assert_eq!(remaining_generations, 1);
    assert!(state_dir.join("notes.json").exists());
    assert_eq!(fs::read_to_string(&log).unwrap().lines().count(), 9);

    fs::remove_dir_all(&root).unwrap();
}
