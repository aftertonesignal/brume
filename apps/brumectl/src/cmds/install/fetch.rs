// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Auto-fetch the brume aarch64 binary from GitHub Releases.
//!
//! brumectl is the host-side companion to brume; it needs an
//! aarch64-unknown-linux-gnu build of the synthesizer to scp onto
//! the CM5 during `brumectl install`. The release pipeline
//! (`release.yml`) already publishes that binary as a release
//! artifact named `brume-aarch64-unknown-linux-gnu`, alongside a
//! `.sha256` sidecar; this module is the consumer.
//!
//! Lookup order, in `binary_path`:
//!
//!   1. `--binary <PATH>` (CLI flag — explicit dev/CI override)
//!   2. `$BRUME_BINARY` (env var — same purpose, set once per shell)
//!   3. Local workspace `target/.../release/brume` (dev cross-build)
//!   4. Cache directory hit (`~/.cache/brumectl/binaries/...`)
//!   5. Download from GitHub Releases (this module)
//!
//! Dev users with a fresh local build keep their fast iteration loop;
//! first-time users get the binary auto-fetched, no
//! "produce-this-yourself" instruction in their face.

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use sha2::{Digest, Sha256};

/// GitHub repository the release artifacts are published to. Held as
/// a constant rather than env-overridable: forks that want to publish
/// their own builds are an edge case that can be addressed if it
/// comes up; for now keeping this hardcoded keeps the install path
/// fail-loud rather than silently fetching from an unintended source.
const REPO_SLUG: &str = "aftertonesignal/brume";

/// Target triple of the aarch64 brume binary. The published release
/// asset is named `brume-<version>-<triple>`: the `release` job in
/// release.yml renames each build-matrix artifact (labelled
/// `brume-<triple>`) to embed the version before uploading. The fetch
/// must request that renamed, versioned name — requesting the bare
/// matrix label 404s against every real release.
const TARGET_TRIPLE: &str = "aarch64-unknown-linux-gnu";

/// HTTP timeout. Generous because release-asset CDN is GitHub Pages
/// fronting, which occasionally takes a few seconds for first-byte;
/// 60 s is "comfortably long for a 5 MB binary on a hotel wifi"
/// without burning the user's afternoon if the host is unreachable.
const HTTP_TIMEOUT: Duration = Duration::from_secs(60);

pub struct Fetcher {
    /// Brume version to fetch — typically `env!("CARGO_PKG_VERSION")`.
    /// brumectl and brume share a workspace version, so a brumectl
    /// pinned at v0.2.0 fetches the v0.2.0 brume binary. That's
    /// almost always what you want; mismatches are easier to debug
    /// when they're explicit.
    pub version: String,
    /// `--no-download` — refuse to fetch over the network. If the
    /// cache is empty and no local build exists, the install fails
    /// rather than silently going online.
    pub no_download: bool,
    /// `--force-download` — re-fetch even if the cache has the
    /// matching version. Used when a release was re-cut under the
    /// same tag (rare, but happens during the initial validation
    /// of a release pipeline).
    pub force_download: bool,
}

impl Fetcher {
    /// Resolve the cache path for the brume binary at this version.
    /// Doesn't create anything; just returns where it *would* be cached.
    pub fn cache_path(&self) -> Result<PathBuf> {
        Ok(Self::cache_dir()?
            .join("binaries")
            .join(format!("brume-v{}-{}", self.version, TARGET_TRIPLE)))
    }

    /// Resolve the cache path for the factory-preset tarball.
    pub fn factory_cache_path(&self) -> Result<PathBuf> {
        Ok(Self::cache_dir()?
            .join("factory")
            .join(format!("brume-v{}-factory.tar.gz", self.version)))
    }

    fn cache_dir() -> Result<PathBuf> {
        Ok(dirs::cache_dir()
            .context("could not resolve user cache directory (no $HOME?)")?
            .join("brumectl"))
    }

    /// Return a path to the brume binary, fetching from GitHub
    /// Releases if necessary. Caches the result so subsequent
    /// installs at the same version skip the network entirely.
    pub fn ensure_local(&self) -> Result<PathBuf> {
        self.fetch_to_cache(&self.asset_url(), self.cache_path()?, true)
    }

    /// Return a path to the factory-preset tarball, fetching from
    /// GitHub Releases if necessary. Same cache + sha256 verification
    /// as `ensure_local`; the tarball isn't marked executable.
    pub fn ensure_factory_local(&self) -> Result<PathBuf> {
        self.fetch_to_cache(&self.factory_asset_url(), self.factory_cache_path()?, false)
    }

    /// Download `url` into `cache_path`, verifying it against the
    /// `<url>.sha256` sidecar, unless the cache already holds it. A
    /// cache hit short-circuits the network; `--no-download` turns a
    /// miss into an error; `--force-download` ignores a present cache.
    /// `executable` chmods the result 0o755 (the binary needs it; the
    /// tarball doesn't). Writes to a `.partial` neighbour then renames,
    /// so a download killed mid-write can't masquerade as a complete
    /// cache entry on the next run.
    fn fetch_to_cache(&self, url: &str, cache_path: PathBuf, executable: bool) -> Result<PathBuf> {
        if cache_path.exists() && !self.force_download {
            return Ok(cache_path);
        }
        if self.no_download {
            bail!(
                "asset not in cache at {} and --no-download was set; \
                 produce it locally or re-run without --no-download",
                cache_path.display()
            );
        }

        let sha_url = format!("{url}.sha256");
        println!("      fetching {url}");
        let body = http_get_bytes(url).context("download release asset")?;

        println!("      verifying sha256 against {sha_url}");
        let sha_text = http_get_text(&sha_url).context("download .sha256 sidecar")?;
        let expected = parse_sha256_sidecar(&sha_text).with_context(|| {
            format!(
                "parse sha256 sidecar (got {} bytes; format expected: '<hex>  <filename>')",
                sha_text.len()
            )
        })?;
        let actual = sha256_hex(&body);
        if !sha_eq_ci(&actual, &expected) {
            bail!("sha256 mismatch for {url}\n  expected {expected}\n  got      {actual}");
        }

        // Cache directory may not exist yet on a fresh install.
        if let Some(parent) = cache_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create cache directory {}", parent.display()))?;
        }

        let tmp = cache_path.with_extension("partial");
        fs::write(&tmp, &body)
            .with_context(|| format!("write {} bytes to {}", body.len(), tmp.display()))?;
        if executable {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755))
                    .with_context(|| format!("chmod 755 {}", tmp.display()))?;
            }
        }
        fs::rename(&tmp, &cache_path)
            .with_context(|| format!("rename {} → {}", tmp.display(), cache_path.display()))?;

        Ok(cache_path)
    }

    fn asset_url(&self) -> String {
        format!(
            "https://github.com/{REPO_SLUG}/releases/download/v{version}/brume-{version}-{TARGET_TRIPLE}",
            version = self.version
        )
    }

    fn factory_asset_url(&self) -> String {
        format!(
            "https://github.com/{REPO_SLUG}/releases/download/v{version}/brume-{version}-factory.tar.gz",
            version = self.version
        )
    }
}

fn http_get_bytes(url: &str) -> Result<Vec<u8>> {
    let agent = ureq::AgentBuilder::new()
        .timeout(HTTP_TIMEOUT)
        .user_agent(concat!("brumectl/", env!("CARGO_PKG_VERSION")))
        .build();
    let resp = agent.get(url).call().map_err(|e| match e {
        ureq::Error::Status(404, _) => anyhow!(
            "{url}: 404. Either the release for this version hasn't been \
             cut yet, or the repo is still private (release assets on a \
             private repo require auth — pass --binary <PATH> for now)"
        ),
        ureq::Error::Status(code, resp) => {
            anyhow!("{url}: HTTP {code} ({})", resp.status_text())
        }
        ureq::Error::Transport(t) => anyhow!("{url}: transport error: {t}"),
    })?;
    let mut buf = Vec::new();
    resp.into_reader()
        .read_to_end(&mut buf)
        .with_context(|| format!("reading body from {url}"))?;
    Ok(buf)
}

fn http_get_text(url: &str) -> Result<String> {
    let bytes = http_get_bytes(url)?;
    String::from_utf8(bytes).with_context(|| format!("body of {url} is not UTF-8"))
}

/// Parse a `sha256sum`-format sidecar line: `<hex>  <filename>` (two
/// spaces between fields, by GNU coreutils convention). Returns just
/// the hex digest; we don't validate the filename column because the
/// release pipeline produces these in a known shape and we already
/// know which URL we requested.
fn parse_sha256_sidecar(text: &str) -> Result<String> {
    let line = text
        .lines()
        .next()
        .ok_or_else(|| anyhow!("empty sha256 sidecar"))?;
    let hex = line
        .split_whitespace()
        .next()
        .ok_or_else(|| anyhow!("malformed sha256 sidecar line: {line:?}"))?;
    if hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("sha256 sidecar token is not a 64-char hex digest: {hex:?}");
    }
    Ok(hex.to_string())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

fn sha_eq_ci(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.chars()
            .zip(b.chars())
            .all(|(x, y)| x.eq_ignore_ascii_case(&y))
}

use std::io::Read;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_path_uses_version_in_filename() {
        let f = Fetcher {
            version: "0.2.0".into(),
            no_download: false,
            force_download: false,
        };
        // We don't assert the exact prefix because dirs::cache_dir
        // is platform-dependent. We only assert the leaf shape.
        let p = f.cache_path().unwrap();
        let leaf = p.file_name().unwrap().to_string_lossy();
        assert_eq!(leaf, "brume-v0.2.0-aarch64-unknown-linux-gnu");
    }

    #[test]
    fn cache_path_includes_brumectl_segment() {
        let f = Fetcher {
            version: "0.2.0".into(),
            no_download: false,
            force_download: false,
        };
        let p = f.cache_path().unwrap();
        let s = p.to_string_lossy();
        assert!(
            s.contains("brumectl"),
            "cache path should be under brumectl/: {s}"
        );
        assert!(
            s.contains("binaries"),
            "cache path should be under .../binaries/: {s}"
        );
    }

    #[test]
    fn asset_url_shape() {
        let f = Fetcher {
            version: "0.3.0".into(),
            no_download: false,
            force_download: false,
        };
        assert_eq!(
            f.asset_url(),
            "https://github.com/aftertonesignal/brume/releases/download/\
             v0.3.0/brume-0.3.0-aarch64-unknown-linux-gnu"
                .replace(' ', "")
        );
    }

    #[test]
    fn factory_asset_url_shape() {
        let f = Fetcher {
            version: "0.3.0".into(),
            no_download: false,
            force_download: false,
        };
        assert_eq!(
            f.factory_asset_url(),
            "https://github.com/aftertonesignal/brume/releases/download/\
             v0.3.0/brume-0.3.0-factory.tar.gz"
                .replace(' ', "")
        );
    }

    #[test]
    fn factory_cache_path_leaf_and_segment() {
        let f = Fetcher {
            version: "0.2.0".into(),
            no_download: false,
            force_download: false,
        };
        let p = f.factory_cache_path().unwrap();
        let leaf = p.file_name().unwrap().to_string_lossy();
        assert_eq!(leaf, "brume-v0.2.0-factory.tar.gz");
        assert!(p.to_string_lossy().contains("brumectl"));
    }

    #[test]
    fn parse_sidecar_extracts_hex_digest() {
        let line = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef  brume-aarch64-unknown-linux-gnu";
        let h = parse_sha256_sidecar(line).unwrap();
        assert_eq!(
            h,
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        );
    }

    #[test]
    fn parse_sidecar_rejects_short_digest() {
        let line = "deadbeef  brume-aarch64-unknown-linux-gnu";
        assert!(parse_sha256_sidecar(line).is_err());
    }

    #[test]
    fn parse_sidecar_rejects_non_hex() {
        let line = "ZZZZ56789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0  brume";
        assert!(parse_sha256_sidecar(line).is_err());
    }

    #[test]
    fn sha256_hex_matches_known_value() {
        // sha256("hello world") = b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9
        let h = sha256_hex(b"hello world");
        assert_eq!(
            h,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn sha_eq_ci_handles_uppercase() {
        assert!(sha_eq_ci("ABCD", "abcd"));
        assert!(sha_eq_ci("dead", "DEAD"));
        assert!(!sha_eq_ci("dead", "beef"));
        assert!(!sha_eq_ci("dead", "dead0"));
    }
}
