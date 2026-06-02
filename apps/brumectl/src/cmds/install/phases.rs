// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Install phases. Each function is a stage in the orchestrator.
//!
//! Phase contract: every phase is idempotent and verbose. Re-running a
//! successful install on the same host is expected (no-op for apt,
//! overwrite-with-same-bytes for configs, daemon-reload for systemd).

use anyhow::{Context, Result, anyhow};

use super::super::ssh;
use super::super::target::Target;
use super::Opts;

/// Runtime-only apt packages brume needs on a Pi OS Lite CM5. `-dev`
/// packages stay off the target — the cross-build handles compile-time
/// linkage.
const APT_PACKAGES: &[&str] = &[
    "libinput-bin",
    "labwc",
    "seatd",
    "swaybg",
    "grim",
    "xwayland",
    "dbus-user-session",
    "fontconfig",
    "fonts-dejavu-core",
    "xkb-data",
    "alsa-utils",
    "libegl1",
    "libgles2",
    // setcap(8) lives here — needed by stage_binary() to grant the
    // brume binary CAP_SYS_NICE so the audio thread can run at
    // SCHED_FIFO. Without the cap audio xruns under sustained UI
    // load. See `reference_audio_rtprio.md`.
    "libcap2-bin",
];

// Appliance config files brume.service + labwc need on the target,
// embedded from deploy/. include_bytes! paths are relative to this file.
const BASH_PROFILE: &[u8] = include_bytes!("../../../../../deploy/config/bash_profile");
const LABWC_AUTOSTART: &[u8] = include_bytes!("../../../../../deploy/config/labwc-autostart");
const LABWC_ENVIRONMENT: &[u8] = include_bytes!("../../../../../deploy/config/labwc-environment");
const BRUME_SERVICE: &[u8] = include_bytes!("../../../../../deploy/systemd/brume.service");
const ASOUND_CONF: &[u8] = include_bytes!("../../../../../deploy/config/asound.conf");

// The transparent cursor theme is harder to embed (77 symlinks + a
// binary cursor). We ship `deploy/scripts/install-transparent-cursor.sh`
// up to the target and invoke it there — it generates the theme from
// scratch via a small python snippet.
const INSTALL_CURSOR_SCRIPT: &[u8] =
    include_bytes!("../../../../../deploy/scripts/install-transparent-cursor.sh");
const BRUME_GADGET_SCRIPT: &[u8] =
    include_bytes!("../../../../../deploy/scripts/brume-gadget-uac2-midi.sh");
const BRUME_GADGET_SERVICE: &[u8] =
    include_bytes!("../../../../../deploy/systemd/brume-gadget.service");

// ─────────────────────────────────────────────────────────────────────
// Phases
// ─────────────────────────────────────────────────────────────────────

pub fn probe(target: &Target) -> Result<()> {
    println!("[1/8] probe");
    let out = ssh::capture_remote(
        target,
        r#"
set -e
echo "host=$(hostname)"
echo "arch=$(uname -m)"
echo "user=$(id -un)"
echo "sudo=$(sudo -n true 2>&1 && echo ok || echo needs-password)"
echo "os_like=$(. /etc/os-release; echo "$ID_LIKE $ID" | tr -s ' ')"
"#,
    )
    .map_err(|e| {
        let msg = e.to_string();
        // ssh exit 255 = connection-level failure (auth, network, DNS).
        // Match on both the exit code tag from capture_remote and the
        // specific stderr patterns ssh emits.
        if msg.contains("Permission denied") {
            anyhow!(
                "SSH to {target} failed — the identity ssh tried did not authenticate.\n  \
                 Try one of:\n    \
                 • ssh-add ~/.ssh/<your-brume-key>       (load into agent)\n    \
                 • export BRUME_SSH_KEY=~/.ssh/<key>     (per-session)\n    \
                 • brumectl -i ~/.ssh/<key> install      (explicit, one-off)\n    \
                 • Add an IdentityFile entry for the host in ~/.ssh/config\n\n  \
                 Underlying: {e}"
            )
        } else if msg.contains("Could not resolve")
            || msg.contains("name resolution")
            || msg.contains("nodename nor servname")
        {
            anyhow!(
                "Could not resolve {target}. Check:\n    \
                 • CM5 is powered + on the LAN (ping brume.local)\n    \
                 • avahi/mDNS is running on the CM5\n    \
                 • or pass an explicit IP: brumectl --host brume@<IP> install\n\n  \
                 Underlying: {e}"
            )
        } else if msg.contains("Connection refused") || msg.contains("No route to host") {
            anyhow!(
                "Could not reach {target}. The CM5 answered DNS/mDNS but not SSH.\n  \
                 Check that sshd is running: `systemctl status ssh` on the device.\n\n  \
                 Underlying: {e}"
            )
        } else {
            e.context("could not probe target")
        }
    })?;

    println!("{}", indent(&out.trim(), "      "));

    let arch = line_value(&out, "arch=").unwrap_or_default();
    if arch != "aarch64" {
        return Err(anyhow!(
            "target arch is {arch:?} but the brume binary is aarch64; \
             brumectl install only supports aarch64 Linux hosts today"
        ));
    }

    let sudo = line_value(&out, "sudo=").unwrap_or_default();
    if sudo != "ok" {
        return Err(anyhow!(
            "target user lacks passwordless sudo (got {sudo:?}); \
             the default Pi OS user has this granted via /etc/sudoers.d — \
             verify `sudo -n true` on the target"
        ));
    }

    let os = line_value(&out, "os_like=").unwrap_or_default();
    if !os.contains("debian") {
        return Err(anyhow!(
            "target OS family is {os:?}, expected Debian-like (Pi OS Lite)"
        ));
    }

    Ok(())
}

pub fn apt_install(target: &Target, opts: &Opts) -> Result<()> {
    println!(
        "[2/8] apt-install runtime dependencies ({} packages)",
        APT_PACKAGES.len()
    );
    if opts.dry_run {
        for p in APT_PACKAGES {
            println!("      (dry-run) would install {p}");
        }
        return Ok(());
    }
    let pkgs = APT_PACKAGES.join(" ");
    ssh::run_remote(
        target,
        &format!(
            "set -e
sudo apt-get update
sudo DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends {pkgs}"
        ),
    )
    .context("apt-get install failed")?;
    Ok(())
}

pub fn stage_binary(target: &Target, opts: &Opts) -> Result<()> {
    println!("[3/8] stage brume binary → /usr/bin/brume");
    let local = binary_path(opts)?;
    let size = std::fs::metadata(&local)?.len();
    println!("      source: {} ({} KB)", local, size / 1024);
    if opts.dry_run {
        println!(
            "      (dry-run) would scp → /tmp + sudo install → /usr/bin/brume + setcap cap_sys_nice"
        );
        return Ok(());
    }
    let tmp = "/tmp/brume-install.bin";
    ssh::scp_push(target, &local, tmp).context("scp brume binary")?;
    // `sudo --` separator: coreutils' `install` takes -o / -g flags,
    // which sudo also claims as "run as user/group". Without the `--`
    // separator sudo steals them and complains with a usage message.
    //
    // Stage the new binary at `/usr/bin/brume.new`, apply CAP_SYS_NICE
    // there, then atomic-mv into place. The previous form chained
    // `install ... && setcap ...` directly onto `/usr/bin/brume`,
    // which meant a setcap failure (ENOTSUP on a future no-xattr
    // rootfs, or any other reason setcap can fail post-install) left
    // the user with the new binary running at SCHED_OTHER and silent
    // xruns under load — the cap requirement isn't observable except
    // through audio glitches that look like a different bug. Staging
    // makes the install atomic from the running brume's POV: setcap
    // either applies before the mv, or the old brume keeps running
    // and the user sees a clear setcap error.
    //
    // CAP_SYS_NICE is an inode xattr, so the mv preserves it. A bare
    // `mv /usr/bin/brume.new /usr/bin/brume` replaces the inode the
    // existing brume process has open without disturbing it (Linux
    // unlink-on-replace).
    //
    // Cleanup: a leftover `/usr/bin/brume.new` from a partial run is
    // harmless (not on PATH; the user can `sudo rm` it). Worth
    // documenting if it ever shows up in support chatter.
    ssh::run_remote(
        target,
        &format!(
            "sudo -- install -m 0755 -o root -g root {tmp} /usr/bin/brume.new \
             && sudo setcap 'cap_sys_nice=eip' /usr/bin/brume.new \
             && sudo mv /usr/bin/brume.new /usr/bin/brume \
             && rm -f {tmp}"
        ),
    )
    .context("install brume binary into /usr/bin")?;
    Ok(())
}

/// Stage the factory preset tree to /usr/share/brume/factory. The brume
/// binary loads these at runtime (they're filesystem files, not embedded
/// in the binary), so without this phase an installed Brume boots with
/// an empty LIBRARY. Runs on both install and `--update` — presets are
/// version-coupled content like the binary, so a refresh should bring
/// new ones.
pub fn stage_factory(target: &Target, opts: &Opts) -> Result<()> {
    println!("[4/8] stage factory presets → /usr/share/brume/factory");
    let local_tar = factory_tarball(opts)?;
    let size = std::fs::metadata(&local_tar)?.len();
    println!("      source: {} ({} KB)", local_tar, size / 1024);
    if opts.dry_run {
        println!(
            "      (dry-run) would scp tarball → /tmp + sudo extract → /usr/share/brume/factory"
        );
        return Ok(());
    }
    let tmp = "/tmp/brume-factory.tar.gz";
    ssh::scp_push(target, &local_tar, tmp).context("scp factory tarball")?;
    // Replace the tree wholesale: it holds only shipped presets — the
    // user's own patches live under the user data dir, never here — so a
    // clean rm + extract is the simplest idempotent install, mirroring
    // deploy-cm5.sh. The tarball's root is the per-engine dirs, so this
    // lands fm/ harmonic/ timbral/ granular/ directly under factory/.
    ssh::run_remote(
        target,
        &format!(
            "sudo rm -rf /usr/share/brume/factory \
             && sudo mkdir -p /usr/share/brume/factory \
             && sudo tar -xzf {tmp} -C /usr/share/brume/factory \
             && rm -f {tmp}"
        ),
    )
    .context("extract factory presets into /usr/share/brume/factory")?;
    Ok(())
}

pub fn deploy_configs(target: &Target, opts: &Opts) -> Result<()> {
    println!("[5/8] deploy appliance configs");

    // User-scoped (~/.bash_profile, ~/.config/...). HOME resolves to
    // the SSH user's home on the target; no path munging needed here.
    push_user_config(target, opts, ".bash_profile", BASH_PROFILE, 0o644)?;
    push_user_config(
        target,
        opts,
        ".config/labwc/autostart",
        LABWC_AUTOSTART,
        0o755,
    )?;
    push_user_config(
        target,
        opts,
        ".config/labwc/environment",
        LABWC_ENVIRONMENT,
        0o644,
    )?;
    push_user_config(
        target,
        opts,
        ".config/systemd/user/brume.service",
        BRUME_SERVICE,
        0o644,
    )?;

    println!("      running install-transparent-cursor.sh on target");
    if opts.dry_run {
        println!("      (dry-run) would push + execute install-transparent-cursor.sh");
    } else {
        let cursor_tmp = "/tmp/brume-install-cursor.sh";
        ssh::push_bytes(target, cursor_tmp, INSTALL_CURSOR_SCRIPT)?;
        ssh::run_remote(target, &format!("bash {cursor_tmp} && rm -f {cursor_tmp}"))
            .context("cursor theme install")?;
    }

    push_root_config(
        target,
        opts,
        "/etc/asound.conf",
        ASOUND_CONF,
        0o644,
        "root",
        "root",
    )?;
    // tty1 autologin is configured by the enable_autologin phase via
    // `raspi-config nonint do_boot_behaviour B2`, which creates the
    // getty drop-in for the install account ($SUDO_USER). We don't
    // ship our own autologin.conf — it would only duplicate that work
    // and hardcode a username.

    if !opts.dry_run {
        ssh::run_remote(target, "systemctl --user daemon-reload")
            .context("systemctl --user daemon-reload")?;
    }
    Ok(())
}

/// Set up the Meridian USB-audio gadget — but only on OTG-capable hardware
/// (a CM5 on an I/O carrier board). A plain Pi 5 has no peripheral USB, so
/// Brume runs without USB audio there and this phase is skipped. Deploys
/// the gadget script + service, adds the dwc2 peripheral dtoverlay to
/// config.txt, and enables the service; it comes up on the post-install
/// reboot. The physical USB_OTG jumper is a hardware step the board guide
/// covers, not something we can automate.
pub fn stage_meridian(target: &Target, opts: &Opts) -> Result<()> {
    println!("[6/8] set up Meridian USB-audio gadget");
    let model = ssh::capture_remote(
        target,
        "tr -d '\\0' < /proc/device-tree/model 2>/dev/null || true",
    )
    .unwrap_or_default();
    let model = model.trim().to_string();
    if !model.contains("Compute Module 5") {
        println!(
            "      board is {model:?} — not CM5/OTG-capable; skipping Meridian \
             (Brume runs, just without USB audio to a DAW)."
        );
        return Ok(());
    }
    println!("      {model} — OTG-capable; installing the gadget.");
    if opts.dry_run {
        println!(
            "      (dry-run) would deploy the gadget script → /usr/local/bin, \
             install + enable brume-gadget.service, and add the dwc2 dtoverlay"
        );
        return Ok(());
    }
    push_root_config(
        target,
        opts,
        "/usr/local/bin/brume-gadget-uac2-midi.sh",
        BRUME_GADGET_SCRIPT,
        0o755,
        "root",
        "root",
    )?;
    push_root_config(
        target,
        opts,
        "/etc/systemd/system/brume-gadget.service",
        BRUME_GADGET_SERVICE,
        0o644,
        "root",
        "root",
    )?;
    // dwc2 peripheral overlay — idempotent append under a [cm5] section so
    // the USB controller comes up in peripheral mode after the reboot.
    ssh::run_remote(
        target,
        "if ! grep -q 'dtoverlay=dwc2,dr_mode=peripheral' /boot/firmware/config.txt; then \
           { echo ''; echo '# Brume Meridian USB gadget (brumectl install)'; \
             echo '[cm5]'; echo 'otg_mode=1'; echo 'dtoverlay=dwc2,dr_mode=peripheral'; } \
           | sudo tee -a /boot/firmware/config.txt >/dev/null; \
         fi",
    )
    .context("add dwc2 peripheral dtoverlay to config.txt")?;
    ssh::run_remote(
        target,
        "sudo systemctl daemon-reload && sudo systemctl enable brume-gadget.service",
    )
    .context("enable brume-gadget.service")?;
    Ok(())
}

pub fn enable_autologin(target: &Target, opts: &Opts) -> Result<()> {
    println!("[7/8] enable tty1 autologin (raspi-config do_boot_behaviour B2)");
    if opts.dry_run {
        println!("      (dry-run) would invoke raspi-config nonint do_boot_behaviour B2");
        return Ok(());
    }
    ssh::run_remote(
        target,
        "sudo raspi-config nonint do_boot_behaviour B2 && sudo systemctl daemon-reload",
    )
    .context("raspi-config autologin")?;
    Ok(())
}

/// Restart the user's brume.service so the new /usr/bin/brume picks
/// up. Only invoked on `--update`: a full install expects a reboot
/// to pull the autologin → labwc → autostart chain end-to-end.
pub fn restart_brume(target: &Target, opts: &Opts) -> Result<()> {
    println!("[7/8] restart brume.service");
    if opts.dry_run {
        println!("      (dry-run) would run `systemctl --user restart brume.service`");
        return Ok(());
    }
    ssh::run_remote(target, "systemctl --user restart brume.service")
        .context("restart brume.service")?;
    Ok(())
}

pub fn verify(target: &Target, opts: &Opts) -> Result<()> {
    println!("[8/8] verify");
    if opts.dry_run {
        println!("      (dry-run) would check brume.service state + /usr/bin/brume");
        return Ok(());
    }
    let out = ssh::capture_remote(
        target,
        r#"
systemctl --user is-enabled brume.service 2>&1 || true
systemctl --user is-active  brume.service 2>&1 || true
ls -la /usr/bin/brume 2>&1 || true
"#,
    )?;
    println!("{}", indent(&out.trim(), "      "));
    // Reboot guidance only makes sense on a full first install — it's
    // the autologin → labwc chain that needs to pick up the new
    // configs. `--update` just restarted brume.service in place,
    // nothing else changed.
    if !opts.update {
        println!();
        println!("Reboot the CM5 to activate autologin → labwc → brume:");
        println!("  ssh {target} sudo reboot");
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────

/// Resolves the cross-built brume binary. Precedence:
///  1. `--binary <PATH>` CLI flag
///  2. `$BRUME_BINARY` environment variable
///  3. Workspace-relative candidate paths (dev workflow)
///
/// The fallback message points the user at the one-command path for
/// re-producing the binary via fly cross-build.
fn binary_path(opts: &Opts) -> Result<String> {
    if let Some(p) = &opts.binary_override {
        if std::path::Path::new(p).exists() {
            return Ok(p.clone());
        }
        return Err(anyhow!("--binary {p} does not exist"));
    }
    if let Ok(env_path) = std::env::var("BRUME_BINARY") {
        if std::path::Path::new(&env_path).exists() {
            return Ok(env_path);
        }
        return Err(anyhow!("$BRUME_BINARY={env_path} does not exist"));
    }
    let candidates = [
        "target/aarch64-unknown-linux-gnu/release/brume",
        "../target/aarch64-unknown-linux-gnu/release/brume",
    ];
    for c in &candidates {
        if std::path::Path::new(c).exists() {
            return Ok((*c).to_string());
        }
    }

    // No local build found — try the user cache (a previous fetch),
    // and fall back to downloading from GitHub Releases. Skipped when
    // --no-download is set.
    let fetcher = super::fetch::Fetcher {
        version: env!("CARGO_PKG_VERSION").to_string(),
        no_download: opts.no_download,
        force_download: opts.force_download,
    };
    let cached = fetcher.ensure_local().with_context(|| {
        format!(
            "could not produce a brume binary. Searched local builds {candidates:?}, \
             then attempted GitHub Releases fetch for v{}. Options:\n  \
             • pass --binary <PATH> (explicit override)\n  \
             • set $BRUME_BINARY=<PATH>\n  \
             • run `cargo build --release --target aarch64-unknown-linux-gnu -p brume-main` \
             (requires the cross-toolchain installed)\n  \
             • re-run with --force-download if the cache is stale",
            env!("CARGO_PKG_VERSION")
        )
    })?;
    Ok(cached.to_string_lossy().into_owned())
}

/// Resolve a local `.tar.gz` of the factory preset tree to stage.
/// Precedence mirrors `binary_path`:
///   1. in-tree `crates/patch-store/factory` (dev workflow) — tarred on
///      the fly so a maintainer install ships their current presets
///   2. the GitHub Releases factory asset (first-time users), cached
///
/// The tar root is the per-engine dirs (fm/ ...), matching both the
/// release.yml bundle and what the patch-store loader expects under the
/// factory root.
fn factory_tarball(opts: &Opts) -> Result<String> {
    let candidates = [
        "crates/patch-store/factory",
        "../crates/patch-store/factory",
    ];
    for dir in candidates {
        if std::path::Path::new(dir).is_dir() {
            let out = std::env::temp_dir().join("brumectl-factory.tar.gz");
            let out = out.to_string_lossy().into_owned();
            let status = std::process::Command::new("tar")
                // COPYFILE_DISABLE stops macOS bsdtar from emitting AppleDouble
                // `._*` sidecars, which would ship junk into the factory tree.
                .env("COPYFILE_DISABLE", "1")
                .args(["-czf", out.as_str(), "-C", dir, "."])
                .status()
                .context("spawn tar to package the in-tree factory")?;
            if !status.success() {
                return Err(anyhow!("tar failed packaging {dir} ({status})"));
            }
            return Ok(out);
        }
    }

    // No in-tree copy — fetch the released tarball (already gzipped).
    let fetcher = super::fetch::Fetcher {
        version: env!("CARGO_PKG_VERSION").to_string(),
        no_download: opts.no_download,
        force_download: opts.force_download,
    };
    let cached = fetcher.ensure_factory_local().with_context(|| {
        format!(
            "could not obtain factory presets. No in-tree {candidates:?}, and \
             the GitHub Releases fetch for v{} failed. Options:\n  \
             • run from a workspace checkout (the in-tree factory is used directly)\n  \
             • re-run without --no-download\n  \
             • re-run with --force-download if the cache is stale",
            env!("CARGO_PKG_VERSION")
        )
    })?;
    Ok(cached.to_string_lossy().into_owned())
}

fn push_user_config(
    target: &Target,
    opts: &Opts,
    relative: &str,
    bytes: &[u8],
    mode: u32,
) -> Result<()> {
    println!(
        "      ~/{} ({} bytes, mode {:#o})",
        relative,
        bytes.len(),
        mode
    );
    if opts.dry_run {
        return Ok(());
    }
    let remote = format!("$HOME/{relative}");
    if let Some(parent) = std::path::Path::new(relative).parent() {
        if !parent.as_os_str().is_empty() {
            ssh::run_remote(target, &format!("mkdir -p $HOME/{}", parent.display()))?;
        }
    }
    // Write + chmod in the same SSH connection so a partial failure
    // can't leave the file at the SSH user's default umask. labwc
    // requires `~/.config/labwc/autostart` to be 0o755; if it lands
    // at 0o644 the GUI silently doesn't launch brume on next boot.
    ssh::push_bytes_with_mode(target, &remote, bytes, mode)?;
    Ok(())
}

fn push_root_config(
    target: &Target,
    opts: &Opts,
    remote: &str,
    bytes: &[u8],
    mode: u32,
    owner: &str,
    group: &str,
) -> Result<()> {
    println!(
        "      {} ({} bytes, mode {:#o}, {}:{})",
        remote,
        bytes.len(),
        mode,
        owner,
        group
    );
    if opts.dry_run {
        return Ok(());
    }
    let stem = std::path::Path::new(remote)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("config");
    let tmp = format!("/tmp/brume-install-{stem}");
    ssh::push_bytes(target, &tmp, bytes)?;
    let parent = std::path::Path::new(remote)
        .parent()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let mkdir = if parent.is_empty() || parent == "/" {
        String::new()
    } else {
        format!("sudo mkdir -p {parent} && ")
    };
    ssh::run_remote(
        target,
        &format!(
            "{mkdir}sudo -- install -m {mode:o} -o {owner} -g {group} {tmp} {remote} && rm -f {tmp}"
        ),
    )?;
    Ok(())
}

fn line_value<'a>(haystack: &'a str, key: &str) -> Option<&'a str> {
    for line in haystack.lines() {
        if let Some(rest) = line.strip_prefix(key) {
            return Some(rest.trim());
        }
    }
    None
}

fn indent(s: &str, prefix: &str) -> String {
    s.lines()
        .map(|l| format!("{prefix}{l}"))
        .collect::<Vec<_>>()
        .join("\n")
}
