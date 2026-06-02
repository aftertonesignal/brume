// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! brumectl — Mac-side companion CLI for the Brume instrument.
//!
//! Bare invocation prints help — every action is explicit. The install
//! subcommand is the primary onboarding path: it takes a CM5 already
//! running Pi OS Lite with SSH access and brings Brume up on top.
//! Substrate provisioning (OS install, hostname, SSH key, network
//! config) is upstream's responsibility, not brumectl's.

use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand};

mod cmds;

/// Brume companion CLI.
#[derive(Debug, Parser)]
#[command(name = "brumectl", version, about, long_about = None)]
struct Cli {
    /// SSH target for the Brume device.
    ///
    /// Defaults to $BRUME_HOST, then "brume@brume.local". Brume's
    /// install paths assume the user account is named `brume`.
    #[arg(long, global = true, value_name = "USER@HOST")]
    host: Option<String>,

    /// SSH identity file. Defaults to $BRUME_SSH_KEY, else SSH agent / ~/.ssh/config.
    #[arg(short = 'i', long, global = true, value_name = "PATH")]
    key: Option<String>,

    #[command(subcommand)]
    cmd: Option<Cmd>,
}

/// `brumectl install` flags.
#[derive(Debug, clap::Args)]
struct InstallArgs {
    /// Print what would be done, do not touch the target.
    ///
    /// Useful for previewing the install against a reachable host
    /// (checks SSH + permissions) without running apt or overwriting
    /// config files.
    #[arg(long)]
    dry_run: bool,

    /// Binary-only refresh path: skip apt + configs, just scp the new
    /// brume binary to /usr/bin/brume and restart brume.service.
    ///
    /// Typical iteration cycle: edit brume source, cross-build,
    /// `brumectl install --update`. Under 30 s per round trip.
    #[arg(long)]
    update: bool,

    /// Path to the cross-built aarch64 brume binary. Overrides auto-detection
    /// and the BRUME_BINARY env var.
    #[arg(long, value_name = "PATH")]
    binary: Option<String>,

    /// Refuse to fetch the brume binary from GitHub Releases when no
    /// local build is found. The install fails fast with a useful
    /// error rather than going online unexpectedly.
    #[arg(long)]
    no_download: bool,

    /// Re-fetch from GitHub Releases even if the user-cache copy at
    /// the matching version already exists. For a release re-cut
    /// under the same tag.
    #[arg(long)]
    force_download: bool,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Install Brume onto a running, network-reachable CM5 over SSH.
    Install(InstallArgs),
    /// Manage Lua scripts on the device (list, push with syntax check).
    #[command(subcommand)]
    Scripts(ScriptsCmd),
    /// Quick health snapshot: firmware, kernel, uptime, temp, memory, network.
    Status,
    /// Open an interactive SSH shell to the device.
    Shell,
    /// Live dashboard — device vitals, console stream, script hot-reload.
    Watch,
    /// (planned) Push an image/app payload update to a connected Brume.
    Update,
    /// (planned) Tail Brume's log output.
    Logs,
    /// (planned) Read/write user config on the device.
    Config,
    /// (planned) Force return to the recovery slot.
    Recover,
    /// (planned) Switch update channel (stable/beta).
    Channel,
    /// (planned) Diagnose common bring-up issues.
    Doctor,
}

/// `brumectl scripts` sub-verbs.
#[derive(Debug, Subcommand)]
enum ScriptsCmd {
    /// List the .lua files currently on the device.
    List,
    /// Copy one or more local .lua files to the device with an atomic
    /// rename and a `luac -p` syntax check. The Script page picks up
    /// new files on its own within ~3 s.
    Push {
        /// Paths to the .lua files to push.
        #[arg(value_name = "FILE", required = true)]
        files: Vec<std::path::PathBuf>,
        /// Skip the `luac -p` pre-push syntax check.
        #[arg(long)]
        no_validate: bool,
    },
    /// Fetch a .lua from the device back to the local workstation.
    Pull {
        /// Script name (with or without .lua extension).
        #[arg(value_name = "NAME")]
        name: String,
        /// Destination path or directory (default: current directory).
        #[arg(long, value_name = "PATH")]
        out: Option<std::path::PathBuf>,
    },
    /// Watch a local .lua and auto-push to the device on every save
    /// (500 ms mtime polling). Pairs with Brume's disk-mtime hot reload
    /// for a "write on the Mac, hear on the CM5" tight loop. Ctrl-C to stop.
    Watch {
        /// Path to the .lua file to watch.
        #[arg(value_name = "FILE")]
        file: std::path::PathBuf,
        /// Skip the `luac -p` syntax check on each save.
        #[arg(long)]
        no_validate: bool,
    },
    /// Ask the engine to load a script by name (must already be on the
    /// device; use `push` first if it isn't).
    Load {
        /// Script name (with or without .lua extension).
        #[arg(value_name = "NAME")]
        name: String,
    },
    /// Ask the engine to unload the currently-loaded script, if any.
    Unload,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let target = cmds::target::Target::resolve(cli.host, cli.key);

    match cli.cmd {
        None => {
            // Explicit-by-default: a bare `brumectl` prints help rather
            // than silently doing anything to a target device.
            Cli::command().print_help()?;
            println!();
            Ok(())
        }
        Some(Cmd::Install(args)) => cmds::install::run(
            &target,
            cmds::install::Opts {
                dry_run: args.dry_run,
                update: args.update,
                binary_override: args.binary,
                no_download: args.no_download,
                force_download: args.force_download,
            },
        ),
        Some(Cmd::Scripts(scripts_cmd)) => {
            let c = match scripts_cmd {
                ScriptsCmd::List => cmds::scripts::Cmd::List,
                ScriptsCmd::Push { files, no_validate } => {
                    cmds::scripts::Cmd::Push { files, no_validate }
                }
                ScriptsCmd::Pull { name, out } => cmds::scripts::Cmd::Pull { name, out },
                ScriptsCmd::Watch { file, no_validate } => {
                    cmds::scripts::Cmd::Watch { file, no_validate }
                }
                ScriptsCmd::Load { name } => cmds::scripts::Cmd::Load { name },
                ScriptsCmd::Unload => cmds::scripts::Cmd::Unload,
            };
            cmds::scripts::run(&target, c)
        }
        Some(Cmd::Status) => cmds::status::run(&target),
        Some(Cmd::Shell) => cmds::shell::run(&target),
        Some(Cmd::Watch) => cmds::watch::run(&target),
        Some(Cmd::Update) => cmds::stub::not_implemented("update"),
        Some(Cmd::Logs) => cmds::stub::not_implemented("logs"),
        Some(Cmd::Config) => cmds::stub::not_implemented("config"),
        Some(Cmd::Recover) => cmds::stub::not_implemented("recover"),
        Some(Cmd::Channel) => cmds::stub::not_implemented("channel"),
        Some(Cmd::Doctor) => cmds::stub::not_implemented("doctor"),
    }
}
