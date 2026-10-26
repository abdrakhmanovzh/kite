//! Self-update from GitHub Releases.
//!
//! The installed app checks the latest release now and then; installing
//! downloads Kite.dmg, copies the app out of it over the running bundle and
//! relaunches once this process has exited. Files Kite downloads itself
//! aren't quarantined, so macOS doesn't ask to "Open Anyway" again.
//!
//! Uses the system's curl, hdiutil and ditto; all blocking, so callers run
//! these off the main thread.

use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;

const LATEST: &str = "https://api.github.com/repos/abdrakhmanovzh/kite/releases/latest";
const ASSET: &str = "Kite.dmg";

/// A newer release than the one running.
#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: String,
    pub download: String,
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    assets: Vec<GithubAsset>,
}

#[derive(Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

/// The installed app bundle this process runs from, if it's an app that
/// can update itself: not a dev build, not run from the disk image or a
/// translocated download.
pub fn installed_bundle() -> Option<PathBuf> {
    use objc2_foundation::NSBundle;
    let path = PathBuf::from(NSBundle::mainBundle().bundlePath().to_string());
    let is_app = path.extension().is_some_and(|e| e == "app");
    let movable =
        !path.starts_with("/Volumes") && !path.to_string_lossy().contains("AppTranslocation");
    (is_app && movable).then_some(path)
}

/// The latest release, when it's newer than this build.
pub fn check() -> Result<Option<Release>> {
    let output = Command::new("/usr/bin/curl")
        .args([
            "-fsSL",
            "--max-time",
            "20",
            "-H",
            "Accept: application/vnd.github+json",
        ])
        .arg(LATEST)
        .stdin(Stdio::null())
        .output()?;
    ensure!(output.status.success(), "couldn't reach GitHub");
    let latest: GithubRelease = serde_json::from_slice(&output.stdout)?;
    let version = latest.tag_name.trim_start_matches('v').to_owned();
    if parse(&version) <= parse(env!("CARGO_PKG_VERSION")) {
        return Ok(None);
    }
    let download = latest
        .assets
        .into_iter()
        .find(|asset| asset.name == ASSET)
        .map(|asset| asset.browser_download_url)
        .context("the release has no Kite.dmg")?;
    Ok(Some(Release { version, download }))
}

fn parse(version: &str) -> Vec<u64> {
    version
        .split('.')
        .map(|part| part.parse().unwrap_or(0))
        .collect()
}

/// Downloads the release and puts its app in place of `bundle`. The
/// running app keeps working (its files stay open); relaunch to switch.
pub fn install(release: &Release, bundle: &Path) -> Result<()> {
    let work = std::env::temp_dir().join(format!("kite-update-{}", release.version));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work)?;
    let dmg = work.join(ASSET);
    run(Command::new("/usr/bin/curl")
        .args(["-fsSL", "--max-time", "900", "-o"])
        .arg(&dmg)
        .arg(&release.download))
    .context("download failed")?;

    let mount = work.join("mount");
    run(Command::new("/usr/bin/hdiutil")
        .args(["attach", "-nobrowse", "-readonly", "-mountpoint"])
        .arg(&mount)
        .arg(&dmg))
    .context("couldn't open the disk image")?;
    let staged = work.join("Kite.app");
    let copied = run(Command::new("/usr/bin/ditto")
        .arg(mount.join("Kite.app"))
        .arg(&staged));
    let _ = run(Command::new("/usr/bin/hdiutil")
        .args(["detach", "-quiet"])
        .arg(&mount));
    copied.context("couldn't copy the new version")?;

    // Swap: the old bundle moves aside (and goes away), the new one takes
    // its place. Moving within the same folder keeps it atomic enough.
    let parent = bundle.parent().context("app has no folder")?;
    let old = parent.join(".Kite-old.app");
    let _ = std::fs::remove_dir_all(&old);
    std::fs::rename(bundle, &old).with_context(|| {
        format!(
            "can't replace {} (move Kite to Applications and try again)",
            bundle.display()
        )
    })?;
    if let Err(error) = run(Command::new("/usr/bin/ditto").arg(&staged).arg(bundle)) {
        // Put the old version back.
        let _ = std::fs::rename(&old, bundle);
        return Err(error.context("couldn't install the new version"));
    }
    let _ = std::fs::remove_dir_all(&old);
    let _ = std::fs::remove_dir_all(&work);
    Ok(())
}

/// Opens `bundle` again once this process has exited, so the new version
/// never shares the Telegram database with the old one.
pub fn relaunch_after_exit(bundle: &Path) -> Result<()> {
    let script = format!(
        "while kill -0 {pid} 2>/dev/null; do sleep 0.2; done; /usr/bin/open \"$0\"",
        pid = std::process::id()
    );
    Command::new("/bin/sh")
        .arg("-c")
        .arg(script)
        .arg(bundle)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(())
}

fn run(command: &mut Command) -> Result<()> {
    let status = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if !status.success() {
        bail!("{:?} exited with {status}", command.get_program());
    }
    Ok(())
}
