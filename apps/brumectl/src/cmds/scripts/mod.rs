// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! `brumectl scripts` — manage Lua scripts on the device.
//!
//! Collapses the "scp and hope" workflow into named verbs:
//!   - `list`  : what's on the device today
//!   - `push`  : atomic-rename a local .lua into ~/brume/scripts/
//!
//! The SSH path is the same as `install` — plain user SSH using the
//! host's OpenSSH + agent + ~/.ssh/config. No engine-side IPC here yet;
//! the Script page's 3 s poll picks up pushed files on its own.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use super::ssh;
use super::target::Target;

const REMOTE_DIR: &str = "~/brume/scripts";

/// Validates that a script *filename* (with the `.lua` suffix) is safe
/// to format into the remote shell paths used by `push_one` /
/// `pull` / `watch` and into the `.cmd` channel consumed by the
/// engine's `load_script`. Mirrors the engine-side rule
/// (`scripting::loader::is_safe_script_name`): each `/`-separated
/// segment of the stem must be ASCII alphanumeric + `-` + `_`,
/// non-empty, ≤64 chars; whole stem ≤252 chars (256 minus `.lua`);
/// trailing `.lua` required.
///
/// Subdirectory paths (`studies/01_metronome.lua`) are accepted so
/// `brumectl scripts load|pull` can target scripts that live under
/// `~/brume/scripts/<subdir>/`. `push` and `watch` only ever see
/// the last path component (via `Path::file_name()`), so they're
/// always called with flat names — but the helper accepts both
/// shapes since it's a single chokepoint.
///
/// Threat model: a maintainer running `brumectl scripts push
/// 'evil; rm -rf ~ .lua'` would otherwise see that name `format!`'d
/// verbatim into `mkdir -p ~/brume/scripts && mv .../tmp <name>` and
/// executed under their SSH session on the device. `ssh.rs` documents
/// "well-behaved paths" as its precondition; this helper is what
/// upholds it for user-supplied names.
fn validated_remote_filename(name: &str) -> anyhow::Result<&str> {
    let stem = name
        .strip_suffix(".lua")
        .ok_or_else(|| anyhow::anyhow!("{name} is not a .lua file — refusing to push"))?;
    if stem.is_empty() || stem.len() > 252 {
        anyhow::bail!("{name}: script filename stem must be 1–252 chars before '.lua'");
    }
    let all_segments_safe = stem.split('/').all(|segment| {
        !segment.is_empty()
            && segment.len() <= 64
            && segment
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    });
    if !all_segments_safe {
        anyhow::bail!(
            "{name}: each '/'-separated segment must be ASCII alphanumeric, '-', \
             or '_' (non-empty, ≤64 chars)"
        );
    }
    Ok(name)
}

pub enum Cmd {
    List,
    Push {
        files: Vec<PathBuf>,
        no_validate: bool,
    },
    Pull {
        name: String,
        out: Option<PathBuf>,
    },
    Watch {
        file: PathBuf,
        no_validate: bool,
    },
    Load {
        name: String,
    },
    Unload,
}

pub fn run(target: &Target, cmd: Cmd) -> Result<()> {
    match cmd {
        Cmd::List => list(target),
        Cmd::Push { files, no_validate } => push(target, &files, no_validate),
        Cmd::Pull { name, out } => pull(target, &name, out.as_deref()),
        Cmd::Watch { file, no_validate } => watch(target, &file, no_validate),
        Cmd::Load { name } => {
            // Engine's load_script takes a stem (no extension). Strip
            // .lua if the user typed it so "push then load" feels the
            // same whether they include it or not, then revalidate
            // through the same gate that push uses — keeps the local
            // failure crisper than waiting for the engine to reject
            // the name on receipt.
            let stem = name.strip_suffix(".lua").unwrap_or(&name);
            let with_ext = format!("{stem}.lua");
            validated_remote_filename(&with_ext)?;
            send_command(target, &format!("load {stem}"))
        }
        Cmd::Unload => send_command(target, "unload"),
    }
}

fn list(target: &Target) -> Result<()> {
    // Recursive find rooted at the scripts dir, stripping the leading
    // `./` so output reads like `foo.lua` for top-level files and
    // `studies/01_metronome.lua` for subdir-organised ones. The
    // engine-side validator accepts both shapes and the SCRIPT
    // page tree shows them as a tree, so the CLI listing must too
    // — earlier `ls -1 {dir}/*.lua | sed 's|^.*/||'` was top-level
    // only and silently hid every subfolder script. Missing dir
    // still treated as "empty" rather than an error.
    //
    // The `cd` makes paths relative so the sed strip is invariant
    // to whatever `~/brume/scripts` expands to on the device. Sort
    // here for deterministic output across find's filesystem-order
    // results.
    let script = format!(
        "mkdir -p {dir} && (cd {dir} && find . -type f -name '*.lua' 2>/dev/null \
         | sed 's|^\\./||' | sort)",
        dir = REMOTE_DIR,
    );
    let out =
        ssh::capture_remote(target, &script).context("failed to list scripts on the device")?;
    let names: Vec<&str> = out
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if names.is_empty() {
        println!("(no scripts in {REMOTE_DIR} on {target})");
    } else {
        println!("scripts on {target}:{REMOTE_DIR}/");
        for n in &names {
            println!("  {n}");
        }
        println!("{} total", names.len());
    }
    Ok(())
}

fn push(target: &Target, files: &[PathBuf], no_validate: bool) -> Result<()> {
    if files.is_empty() {
        anyhow::bail!("scripts push: no files specified");
    }

    // Validate all files up front so a syntax error in file 3 doesn't
    // land files 1 and 2 first.
    if !no_validate {
        for path in files {
            validate_lua(path)?;
        }
    }

    for path in files {
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .with_context(|| format!("invalid filename: {}", path.display()))?;
        validated_remote_filename(name).with_context(|| "scripts push")?;

        print!("pushing {name} ... ");
        // Already-validated, so skip the second luac call.
        push_one(target, path, name, /* no_validate = */ true)
            .with_context(|| format!("push {name} failed"))?;
        println!("ok");
    }

    println!();
    println!(
        "✓ {} script{} pushed to {target}:{REMOTE_DIR}/",
        files.len(),
        if files.len() == 1 { "" } else { "s" },
    );
    println!("  Script page picks up new files within ~3 s.");
    Ok(())
}

fn pull(target: &Target, name: &str, out: Option<&Path>) -> Result<()> {
    // Accept both "foo" and "foo.lua" — the device-side filename always
    // carries the extension, so we tack it on if the user didn't.
    let filename = if name.ends_with(".lua") {
        name.to_string()
    } else {
        format!("{name}.lua")
    };
    validated_remote_filename(&filename).with_context(|| "scripts pull")?;

    // Destination: default to cwd/<filename>. If --out was given,
    // detect whether it's a directory or a full path. A trailing '/'
    // or an existing directory means "drop the file inside here"; any
    // other path is treated as the exact target.
    let dest: PathBuf = match out {
        Some(p) if p.is_dir() || p.to_string_lossy().ends_with('/') => p.join(&filename),
        Some(p) => p.to_path_buf(),
        None => PathBuf::from(&filename),
    };

    let remote_path = format!("{REMOTE_DIR}/{filename}");
    let dest_str = dest
        .to_str()
        .with_context(|| format!("non-utf8 destination: {}", dest.display()))?;

    println!("pulling {filename} ← {target}:{remote_path}");
    ssh::scp_pull(target, &remote_path, dest_str)
        .with_context(|| format!("scp {target}:{remote_path} → {dest_str} failed"))?;
    println!("✓ wrote {}", dest.display());
    Ok(())
}

/// Watch a local .lua for mtime changes and re-push on every save.
///
/// Uses 500 ms polling rather than fsevents/inotify — portable, no
/// extra deps, and the cadence is plenty fast for a human saving a
/// file. First iteration always pushes so the initial state lines up.
/// Ctrl-C exits the loop.
fn watch(target: &Target, file: &Path, no_validate: bool) -> Result<()> {
    let name = file
        .file_name()
        .and_then(|s| s.to_str())
        .with_context(|| format!("invalid filename: {}", file.display()))?;
    validated_remote_filename(name).with_context(|| "scripts watch")?;
    if !file.exists() {
        anyhow::bail!("scripts watch: {} does not exist", file.display());
    }

    println!(
        "watching {} (Ctrl-C to stop) — repushing to {target}:{REMOTE_DIR}/{name} on save",
        file.display(),
    );

    let mut last_mtime: Option<std::time::SystemTime> = None;
    loop {
        let mtime = std::fs::metadata(file)
            .and_then(|m| m.modified())
            .with_context(|| format!("failed to stat {}", file.display()))?;
        if Some(mtime) != last_mtime {
            // First iteration always pushes (last_mtime is None). After
            // that, only changes trigger. Print the timestamp so the
            // watch log reads cleanly in a terminal the user has
            // running alongside their editor.
            let ts = chrono_like_hhmmss();
            print!("[{ts}] {name} changed — ");
            match push_one(target, file, name, no_validate) {
                Ok(()) => println!("pushed"),
                Err(e) => {
                    // Don't bail on a transient push failure (user saved
                    // a broken file, network blip, etc.) — print the
                    // error and keep watching. The next good save
                    // recovers.
                    println!("push failed: {e:#}");
                }
            }
            last_mtime = Some(mtime);
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
}

fn push_one(target: &Target, file: &Path, name: &str, no_validate: bool) -> Result<()> {
    if !no_validate {
        validate_lua(file)?;
    }
    let local = file
        .to_str()
        .with_context(|| format!("non-utf8 path: {}", file.display()))?;
    let tmp_remote = format!("{REMOTE_DIR}/{name}.tmp");
    let final_remote = format!("{REMOTE_DIR}/{name}");
    ssh::run_remote(target, &format!("mkdir -p {REMOTE_DIR}"))
        .context("failed to ensure remote scripts dir exists")?;
    ssh::scp_push(target, local, &tmp_remote)
        .with_context(|| format!("scp {name} → {target}:{tmp_remote} failed"))?;
    ssh::run_remote(target, &format!("mv {tmp_remote} {final_remote}"))
        .with_context(|| format!("atomic rename failed on {target}"))?;
    Ok(())
}

/// Format the current local time as HH:MM:SS. chrono-free because we
/// don't already depend on it and it's a single stamp per event.
fn chrono_like_hhmmss() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Local time-of-day in seconds since midnight. TZ offset isn't
    // worth the dependency — "what time is it right now on this
    // terminal" is what the user actually wants, and tm_gmtoff would
    // require libc bindings. Using UTC stamp is close enough for a
    // watch log.
    let sod = now % 86_400;
    let h = sod / 3600;
    let m = (sod % 3600) / 60;
    let s = sod % 60;
    format!("{h:02}:{m:02}:{s:02}Z")
}

/// Drop a one-line command into `~/brume/scripts/.cmd` and poll the
/// device for the matching `.cmd.result`. The engine picks up the
/// command on its ~500 ms polling tick and writes back `ok` or
/// `err <message>`. Times out after 3 s (engine down / script thread
/// stuck).
fn send_command(target: &Target, cmd: &str) -> Result<()> {
    let cmd_path = format!("{REMOTE_DIR}/.cmd");
    let result_path = format!("{REMOTE_DIR}/.cmd.result");

    // Clear any stale result from a previous run so we don't misread
    // it as a response to this command. Also ensure the dir exists.
    ssh::run_remote(
        target,
        &format!("mkdir -p {REMOTE_DIR} && rm -f {result_path}"),
    )
    .context("failed to prep remote command state")?;

    // Write the command file. Use printf to avoid shell interpolation
    // gotchas in the command string (names with spaces, etc.). We
    // quote-escape the caller-supplied cmd since it may contain user
    // input (script name), although in practice it's restricted.
    ssh::run_remote(
        target,
        &format!("printf '%s\\n' {} > {cmd_path}", shell_single_quote(cmd)),
    )
    .context("failed to write command file")?;

    println!("→ {target}: {cmd}");

    // Poll the result file. `cat` returns exit 0 with content when it
    // exists, nonzero otherwise; loop with a short sleep.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while std::time::Instant::now() < deadline {
        // Try to read the result; if it's not there yet, wait a tick.
        if let Ok(out) = ssh::capture_remote(
            target,
            &format!("cat {result_path} 2>/dev/null && rm -f {result_path}"),
        ) {
            let line = out.trim();
            if !line.is_empty() {
                if let Some(err) = line.strip_prefix("err ") {
                    anyhow::bail!("engine rejected command: {err}");
                }
                if line == "ok" {
                    println!("✓ engine acknowledged");
                    return Ok(());
                }
                // Unknown response — surface it raw rather than silently
                // succeed.
                anyhow::bail!("unexpected result from engine: {line}");
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    anyhow::bail!(
        "no response from engine within 3 s — is brume.service running? \
         (check with `brumectl status`)"
    )
}

/// Single-quote a string for safe inclusion in a remote bash command
/// argument. Mirrors the quoting in cmds::ssh but lives here so this
/// module doesn't need to reach into ssh's private helpers.
fn shell_single_quote(s: &str) -> String {
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

/// Syntax-check a local .lua via `luac -p` if available. Returns Ok
/// when the file parses OR when `luac` isn't installed (we'd rather
/// push than block on a missing tool; print a one-line note so the
/// user knows they're skipping the gate).
fn validate_lua(path: &Path) -> Result<()> {
    match which_luac() {
        None => {
            eprintln!(
                "(luac not found on PATH — skipping syntax check for {}; install Lua to enable)",
                path.display(),
            );
            Ok(())
        }
        Some(luac) => {
            let out = std::process::Command::new(&luac)
                .arg("-p")
                .arg(path)
                .output()
                .with_context(|| format!("failed to invoke {}", luac.display()))?;
            if out.status.success() {
                return Ok(());
            }
            let stderr = String::from_utf8_lossy(&out.stderr);
            anyhow::bail!(
                "syntax check failed for {}:\n  {}",
                path.display(),
                stderr.trim().lines().next().unwrap_or("(no message)"),
            );
        }
    }
}

/// Returns the path to a usable `luac` if one is on PATH.
///
/// Checks both `luac` (unversioned, Debian/macOS) and `luac5.4` / `luac5.3`
/// (Debian names the versioned binary only). Returns the first one that
/// exists. Avoids a second shell-out per file by caching via OnceLock
/// would be nice, but this only runs once per `push` invocation.
fn which_luac() -> Option<PathBuf> {
    for candidate in ["luac", "luac5.4", "luac5.3", "luac5.1"] {
        let out = std::process::Command::new("which")
            .arg(candidate)
            .output()
            .ok()?;
        if out.status.success() {
            let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !path.is_empty() {
                return Some(PathBuf::from(path));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::validated_remote_filename;

    #[test]
    fn accepts_typical_filenames() {
        for ok in [
            "diffuser.lua",
            "study_01_metronome.lua",
            "clock-driver.lua",
            "FX42.lua",
            "_underscore.lua",
            // Subdirectory layouts — `brumectl scripts pull|load`
            // accept these so users can target scripts that live
            // under ~/brume/scripts/<subdir>/.
            "studies/01_metronome.lua",
            "user_scripts/drone.lua",
            "deep/nested/path.lua",
        ] {
            validated_remote_filename(ok).unwrap_or_else(|e| panic!("{ok} should pass: {e:#}"));
        }
    }

    #[test]
    fn rejects_shell_metachars_and_traversal() {
        // Each of these would otherwise land in `format!("mv tmp {name}")`
        // and execute under SSH on the device.
        let bad = [
            "evil; rm -rf ~ .lua",
            "../../etc/passwd.lua",
            "studies/../leak.lua", // mid-path traversal
            "/absolute/path.lua",  // empty first segment
            "studies//double.lua", // empty middle segment
            "trailing/.lua",       // empty trailing segment (stem ends with /)
            "name`whoami`.lua",
            "name$(injection).lua",
            "name|pipe.lua",
            "name with spaces.lua",
            "name.with.dots.lua",
            r"back\slash.lua",
            "café.lua", // non-ASCII alphanumeric
            ".lua",     // empty stem
            "no_extension",
            "wrong.txt",
        ];
        for n in bad {
            assert!(
                validated_remote_filename(n).is_err(),
                "{n} should be rejected"
            );
        }
    }

    #[test]
    fn rejects_oversize_stem() {
        let stem = "a".repeat(65);
        let name = format!("{stem}.lua");
        assert!(validated_remote_filename(&name).is_err());
        let stem = "a".repeat(64);
        let name = format!("{stem}.lua");
        validated_remote_filename(&name).unwrap();
    }
}
