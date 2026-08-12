use crate::wire::DeployRequest;
#[cfg(test)]
use crate::wire::Response;
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const REMOTE_COMMAND: &str = "exec \"$HOME/.local/bin/buzz-backend-host\" remote-deploy";
const STDOUT_CAP: usize = 1024 * 1024;
const STDERR_CAP: usize = 64 * 1024;

enum Stream {
    Stdout,
    Stderr,
}

pub fn deploy(host: &str, request: &DeployRequest) -> Result<String, String> {
    let payload = serde_json::to_vec(request).map_err(|_| "could not encode deploy request")?;
    let mut child = Command::new("ssh")
        .args([
            "-T",
            "-o",
            "BatchMode=yes",
            "-o",
            "ClearAllForwardings=yes",
            "-o",
            "ExitOnForwardFailure=yes",
            "-o",
            "ForwardAgent=no",
            "-o",
            "ConnectTimeout=10",
            "-o",
            "ServerAliveInterval=15",
            "-o",
            "ServerAliveCountMax=3",
            "--",
            host,
            REMOTE_COMMAND,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not start ssh: {e}"))?;
    let stdin_result = child
        .stdin
        .take()
        .ok_or_else(|| "could not open ssh stdin".to_string())
        .and_then(|mut stdin| {
            stdin
                .write_all(&payload)
                .map_err(|e| format!("could not send deploy request to host: {e}"))
        });
    if let Err(error) = stdin_result {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }

    let (sender, receiver) = mpsc::channel();
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "could not open ssh stdout".to_string())?;
    spawn_reader(stdout, STDOUT_CAP, Stream::Stdout, sender.clone());
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "could not open ssh stderr".to_string())?;
    spawn_reader(stderr, STDERR_CAP, Stream::Stderr, sender);

    let deadline = Instant::now() + Duration::from_secs(600);
    let mut stdout = None;
    let mut stderr = None;
    loop {
        while let Ok((stream, bytes)) = receiver.try_recv() {
            store_or_terminate(&mut child, stream, bytes, &mut stdout, &mut stderr)?;
        }
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("host deploy timed out after 600 seconds".into());
            }
            Err(e) => return Err(format!("could not wait for ssh: {e}")),
        }
    }

    let drain_deadline = Instant::now() + Duration::from_secs(2);
    while stdout.is_none() || stderr.is_none() {
        let remaining = drain_deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("could not finish reading ssh output".into());
        }
        let (stream, bytes) = receiver
            .recv_timeout(remaining)
            .map_err(|_| "could not finish reading ssh output".to_string())?;
        store_or_terminate(&mut child, stream, bytes, &mut stdout, &mut stderr)?;
    }
    let status = child
        .wait()
        .map_err(|e| format!("could not collect ssh status: {e}"))?;
    let stdout = stdout.unwrap_or_default();
    let stderr = stderr.unwrap_or_default();
    if !status.success() {
        let stderr = String::from_utf8_lossy(&stderr);
        return Err(format!("ssh deploy failed: {}", stderr.trim()));
    }
    let value: serde_json::Value = serde_json::from_slice(&stdout)
        .map_err(|_| "remote host returned an invalid response".to_string())?;
    if value.get("ok") != Some(&serde_json::Value::Bool(true)) {
        return Err(value
            .get("error")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("remote host refused the deploy")
            .to_string());
    }
    value
        .get("agent_id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "remote host response omitted agent_id".to_string())
}

fn spawn_reader(
    reader: impl Read + Send + 'static,
    cap: usize,
    stream: Stream,
    sender: mpsc::Sender<(Stream, Result<Vec<u8>, String>)>,
) {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = reader
            .take((cap + 1) as u64)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
            .map_err(|e| format!("could not read ssh output: {e}"));
        let _ = sender.send((stream, result));
    });
}

fn store_stream(
    stream: Stream,
    bytes: Vec<u8>,
    stdout: &mut Option<Vec<u8>>,
    stderr: &mut Option<Vec<u8>>,
) -> Result<(), String> {
    let (cap, slot) = match stream {
        Stream::Stdout => (STDOUT_CAP, stdout),
        Stream::Stderr => (STDERR_CAP, stderr),
    };
    if bytes.len() > cap {
        return Err("ssh output exceeded the provider limit".into());
    }
    *slot = Some(bytes);
    Ok(())
}

fn store_or_terminate(
    child: &mut std::process::Child,
    stream: Stream,
    bytes: Result<Vec<u8>, String>,
    stdout: &mut Option<Vec<u8>>,
    stderr: &mut Option<Vec<u8>>,
) -> Result<(), String> {
    let result = bytes.and_then(|bytes| store_stream(stream, bytes, stdout, stderr));
    if let Err(error) = result {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_command_is_fixed_and_contains_no_payload() {
        assert_eq!(
            REMOTE_COMMAND,
            "exec \"$HOME/.local/bin/buzz-backend-host\" remote-deploy"
        );
        assert!(!REMOTE_COMMAND.contains("BUZZ_"));
    }

    #[test]
    fn response_shape_matches_the_provider_protocol() {
        let value = serde_json::to_value(Response::deployed("unit.service")).unwrap();
        assert_eq!(value["agent_id"], "unit.service");
    }
}
