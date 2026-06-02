// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Thin wrappers around `ssh` / `scp` for the install orchestrator.
//!
//! Design: shell out to the host's OpenSSH rather than pulling in
//! libssh/rust-ssh. The user already has their agent + keys configured;
//! shelling out inherits every customization they've already made in
//! `~/.ssh/config` and is trivial to debug (run the same command by
//! hand). Also keeps the binary small.
//!
//! Every remote command:
//! - runs non-interactively (`-o BatchMode=yes`) so a missing key fails
//!   fast instead of hanging on a password prompt
//! - honors `Target.key_args()` to inject `-i <path>` when the caller
//!   wants an explicit identity file
//! - pipes stdin/stdout/stderr through so apt progress etc. is live

use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, anyhow};

use super::target::Target;

/// Run a remote shell command and wait for it to finish. Inherits
/// stdin/stdout/stderr so the user sees live output.
///
/// `script` is passed verbatim to the remote `/bin/bash -lc`, so it can
/// contain pipelines, redirects, `&&`, etc.
pub fn run_remote(target: &Target, script: &str) -> Result<()> {
    let mut cmd = ssh_command(target);
    cmd.arg(wrap_bash_lc(script));
    let status = cmd
        .status()
        .with_context(|| format!("failed to invoke ssh for {target} (is the host reachable?)"))?;
    if !status.success() {
        return Err(anyhow!(
            "remote command failed ({}): {}",
            status.code().unwrap_or(-1),
            script.lines().next().unwrap_or("").trim()
        ));
    }
    Ok(())
}

/// Run a remote command and capture stdout. Stderr is captured
/// alongside so error paths can surface the actual ssh/shell message
/// (auth failures land on stderr); successful runs only return stdout.
pub fn capture_remote(target: &Target, script: &str) -> Result<String> {
    let mut cmd = ssh_command(target);
    cmd.arg(wrap_bash_lc(script));
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    let out = cmd
        .output()
        .with_context(|| format!("failed to invoke ssh for {target}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let code = out.status.code().unwrap_or(-1);
        return Err(anyhow!(
            "ssh exit {}: {}",
            code,
            stderr.trim().lines().next().unwrap_or("(no stderr)")
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Stream bytes to a remote file by piping through `ssh`. Useful when
/// the source is an embedded byte slice (we'd rather not write it to a
/// local tmpfile first and then invoke `scp`).
///
/// `remote_path` is passed verbatim to bash, so absolute paths like
/// `/tmp/brume-install-<nonce>` work without escaping. For writes to
/// root-owned paths the caller should stage into `/tmp` first and then
/// `run_remote(target, "sudo install ...")` to the final location.
pub fn push_bytes(target: &Target, remote_path: &str, bytes: &[u8]) -> Result<()> {
    push_bytes_inner(target, remote_path, bytes, None)
}

/// Like [`push_bytes`] but applies a chmod in the same SSH connection.
/// Eliminates the two-round-trip race where a write succeeds but a
/// follow-up `chmod` fails on a network blip — leaving a config file
/// with the SSH user's umask permissions instead of the requested
/// mode. labwc-autostart at 0644 instead of 0755 silently breaks
/// next-boot launch; this helper closes that gap.
pub fn push_bytes_with_mode(
    target: &Target,
    remote_path: &str,
    bytes: &[u8],
    mode: u32,
) -> Result<()> {
    push_bytes_inner(target, remote_path, bytes, Some(mode))
}

fn push_bytes_inner(
    target: &Target,
    remote_path: &str,
    bytes: &[u8],
    mode: Option<u32>,
) -> Result<()> {
    // `remote_path` is inserted verbatim so `$HOME/...` expands on the
    // remote; our install paths are all well-behaved (no spaces, no
    // shell metacharacters) so we don't need single-quote protection.
    let script = match mode {
        Some(m) => format!("cat > {remote_path} && chmod {m:o} {remote_path}"),
        None => format!("cat > {remote_path}"),
    };
    let mut cmd = ssh_command(target);
    cmd.arg(wrap_bash_lc(&script));
    cmd.stdin(Stdio::piped());
    let mut child = cmd
        .spawn()
        .with_context(|| format!("failed to invoke ssh for {target}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(bytes)
            .with_context(|| format!("failed to write bytes to remote {remote_path}"))?;
    }
    let status = child.wait().context("ssh wait failed")?;
    if !status.success() {
        return Err(anyhow!(
            "remote write to {remote_path} failed (exit {})",
            status.code().unwrap_or(-1)
        ));
    }
    Ok(())
}

/// Copy a local file to a remote path via `scp`. Used for large binary
/// payloads (brume itself); smaller configs go through `push_bytes`.
pub fn scp_push(target: &Target, local_path: &str, remote_path: &str) -> Result<()> {
    let mut cmd = Command::new("scp");
    cmd.args(target.key_args())
        .arg("-o")
        .arg("BatchMode=yes")
        .arg(local_path)
        .arg(format!("{}:{}", target.ssh_target, remote_path));
    let status = cmd.status().with_context(|| {
        format!("failed to invoke scp for {local_path} → {target}:{remote_path}")
    })?;
    if !status.success() {
        return Err(anyhow!(
            "scp {local_path} → {remote_path} failed (exit {})",
            status.code().unwrap_or(-1)
        ));
    }
    Ok(())
}

/// Copy a remote file to a local path via `scp`. Mirror of `scp_push`.
pub fn scp_pull(target: &Target, remote_path: &str, local_path: &str) -> Result<()> {
    let mut cmd = Command::new("scp");
    cmd.args(target.key_args())
        .arg("-o")
        .arg("BatchMode=yes")
        .arg(format!("{}:{}", target.ssh_target, remote_path))
        .arg(local_path);
    let status = cmd.status().with_context(|| {
        format!("failed to invoke scp for {target}:{remote_path} → {local_path}")
    })?;
    if !status.success() {
        return Err(anyhow!(
            "scp {remote_path} → {local_path} failed (exit {})",
            status.code().unwrap_or(-1)
        ));
    }
    Ok(())
}

fn ssh_command(target: &Target) -> Command {
    let mut cmd = Command::new("ssh");
    cmd.args(target.key_args())
        .arg("-o")
        .arg("BatchMode=yes")
        .arg(&target.ssh_target);
    cmd
}

/// Build the single ssh remote-command string that invokes `bash -c
/// <script>` while surviving ssh's argv-join semantics.
///
/// `-c` rather than `-lc` on purpose: `-l` makes bash source
/// `/etc/profile` + `~/.profile`, which on Pi OS Lite prints PAM
/// notices (rfkill country warning etc.) on every invocation. Our
/// commands don't need login-shell environment — sudo has its own
/// secure_path for root commands, and all binaries we invoke live in
/// standard /usr/bin locations. Skipping -l gives clean output.
///
/// When std::process::Command passes multiple args to ssh (e.g.
/// `cmd.arg("bash").arg("-c").arg(script)`), ssh concatenates them
/// with spaces before sending one string to the remote shell. The
/// remote sh -c then re-parses that joined string: `bash -c sudo foo`
/// becomes bash with `-c sudo foo`, i.e. bash runs `sudo` as its -c
/// argument and `foo` becomes a positional arg — the actual command
/// gets silently dropped.
///
/// Wrapping the script in single quotes (with inner `'` escaped as
/// `'\''`) and passing the whole thing as ONE arg to ssh preserves the
/// boundary: remote sh -c sees `bash -c 'script'` and hands the whole
/// quoted block to bash intact.
fn wrap_bash_lc(script: &str) -> String {
    format!("bash -c {}", shell_quote(script))
}

/// Single-quote a path for safe use inside a bash command. Only
/// necessary for paths that contain spaces or shell metacharacters;
/// our install paths are all well-behaved, but quoting is cheap and
/// future-proofs against a user home with a space in it.
fn shell_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if c == '\'' {
            out.push_str(r"'\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}
