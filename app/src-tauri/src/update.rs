//! App updates (ADR 0014). "Check for updates" in Settings reads `latest.json` from the newest
//! stable release on GitHub. Installing downloads this platform's build, checks it was signed
//! with the update signing key, and puts it in place of this one:
//!
//! - Windows: the portable exe swaps itself (a running exe can be renamed) and starts again.
//! - Debian: `apt-get install` of the new `.deb` behind the system's password prompt, then a restart.
//! - Android: the APK is handed to Android's installer (MainActivity.kt), where the Author taps Install.

use base64::Engine;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;
#[cfg(any(windows, target_os = "android"))]
use std::path::Path;
use std::time::Duration;

/// GitHub answers this with the newest published release that isn't a pre-release.
const MANIFEST_URL: &str = "https://github.com/Fiehn/Wrtier/releases/latest/download/latest.json";
const RELEASE_PAGE: &str = "https://github.com/Fiehn/Wrtier/releases/tag/v";

/// The public half of the update signing key (`tauri signer generate`). The private half is the
/// repository secret the release workflow signs with. A build signed with any other key is refused.
const PUBLIC_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDYxRENGODBBNzM3RTYwOUQKUldTZFlINXpDdmpjWWFQblpxLzJSRGtWTGtxaUhXQmlYWkNkaGducUk2dFQ5UFpwd1hJY0xmRmgK";

/// No build of the app comes near this; a download that does is refused rather than filling memory.
const MAX_DOWNLOAD: u64 = 512 * 1024 * 1024;

const BAD_SIGNATURE: &str = "The download isn't signed with wrtier's update key, so it wasn't installed.";

/// `latest.json`, as the release workflow writes it.
#[derive(Deserialize)]
struct Manifest {
    version: String,
    #[serde(default)]
    platforms: HashMap<String, Build>,
}

/// One platform's file in a release, and its signature (the `.sig` file's contents).
#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct Build {
    pub url: String,
    pub signature: String,
}

/// What a check found.
#[derive(Debug, PartialEq)]
pub enum Found {
    /// No stable App version newer than this one.
    Newest,
    Available { version: String, build: Build },
    /// A newer App version without a build for this platform (yet).
    NoBuild { version: String },
}

/// This platform's key in `latest.json`, or None where the app can't update itself.
pub fn target() -> Option<&'static str> {
    if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        Some("windows-x86_64")
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("linux-x86_64")
    } else if cfg!(all(target_os = "android", target_arch = "aarch64")) {
        Some("android-aarch64")
    } else {
        None
    }
}

/// The release's page on GitHub, for "What's new".
pub fn release_page(version: &str) -> String {
    format!("{RELEASE_PAGE}{version}")
}

/// Reads `latest.json` and decides whether it offers this Device an update. Only a stable App
/// version newer than this one counts (GitHub's latest release is never a pre-release, but a
/// hand-edited manifest could say otherwise).
pub fn pick(manifest: &str, current: &semver::Version, target: &str) -> Result<Found, String> {
    let manifest: Manifest =
        serde_json::from_str(manifest).map_err(|e| format!("The release's list of updates can't be read: {e}"))?;
    let version = semver::Version::parse(manifest.version.trim_start_matches('v'))
        .map_err(|e| format!("The release's version \"{}\" can't be read: {e}", manifest.version))?;
    if !version.pre.is_empty() || version <= *current {
        return Ok(Found::Newest);
    }
    let version = version.to_string();
    Ok(match manifest.platforms.get(target) {
        Some(build) => Found::Available { version, build: build.clone() },
        None => Found::NoBuild { version },
    })
}

/// Checks that `data` was signed with the key whose public half is `public_key`, for `version`.
/// The signature and key are base64 of minisign's text files, as `tauri signer` writes them.
///
/// The release workflow signs with `--app-version`, which puts "version:0.2.0" in the signed
/// comment. `latest.json` itself isn't signed, so without that check a tampered one could offer
/// an older (genuinely signed) build under a newer version number.
pub fn verify_with(data: &[u8], signature: &str, public_key: &str, version: &str) -> Result<(), String> {
    let text = |b64: &str| {
        base64::engine::general_purpose::STANDARD.decode(b64.trim()).ok().and_then(|bytes| String::from_utf8(bytes).ok())
    };
    let key = text(public_key)
        .and_then(|t| minisign_verify::PublicKey::decode(&t).ok())
        .ok_or("The app's update key can't be read.")?;
    let signature = text(signature).and_then(|t| minisign_verify::Signature::decode(&t).ok()).ok_or(BAD_SIGNATURE)?;
    key.verify(data, &signature, false).map_err(|_| BAD_SIGNATURE.to_string())?;
    let signed_for = signature.trusted_comment().split('\t').find_map(|field| field.strip_prefix("version:"));
    if signed_for != Some(version) {
        return Err(format!("The download isn't signed as version {version}, so it wasn't installed."));
    }
    Ok(())
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .user_agent("wrtier-updater")
        .build()
        .into()
}

fn http_error(e: ureq::Error) -> String {
    match e {
        ureq::Error::HostNotFound | ureq::Error::ConnectionFailed | ureq::Error::Timeout(_) => {
            "Couldn't reach GitHub. Check the internet connection and try again.".into()
        }
        ureq::Error::StatusCode(404) => "GitHub has no release with updates yet.".into(),
        e => format!("Couldn't reach GitHub: {e}"),
    }
}

/// Looks for an update on GitHub.
pub fn check(current: &semver::Version) -> Result<Found, String> {
    let Some(target) = target() else {
        return Err("The app can't update itself on this platform.".into());
    };
    let mut response = agent().get(MANIFEST_URL).call().map_err(http_error)?;
    let manifest = response.body_mut().read_to_string().map_err(http_error)?;
    pick(&manifest, current, target)
}

/// Downloads a build and checks its signature. `progress` gets the bytes so far and the total,
/// when GitHub says it.
pub fn download(build: &Build, version: &str, mut progress: impl FnMut(u64, Option<u64>)) -> Result<Vec<u8>, String> {
    let response = agent().get(&build.url).call().map_err(http_error)?;
    let total = response.body().content_length();
    if total.is_some_and(|t| t > MAX_DOWNLOAD) {
        return Err("The download is far bigger than the app, so it wasn't downloaded.".into());
    }
    let mut reader = response.into_body().into_with_config().limit(MAX_DOWNLOAD).reader();
    let mut bytes = Vec::with_capacity(total.unwrap_or(0) as usize);
    let mut chunk = vec![0; 256 * 1024];
    loop {
        let n = reader.read(&mut chunk).map_err(|e| format!("The download stopped: {e}"))?;
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..n]);
        progress(bytes.len() as u64, total);
    }
    verify_with(&bytes, &build.signature, PUBLIC_KEY, version)?;
    Ok(bytes)
}

/// Where the new exe is written before it takes the running one's place: next to it, so the
/// swap is a rename within one folder.
#[cfg(windows)]
fn staged(exe: &Path) -> PathBuf {
    exe.with_extension("update.exe")
}

/// Whether this copy can update itself, checked before downloading. Returns the path to start
/// again once it's done.
#[cfg(windows)]
pub fn ready() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let probe = staged(&exe);
    std::fs::write(&probe, b"").map_err(|_| {
        let folder = exe.parent().map(|p| p.display().to_string()).unwrap_or_default();
        format!("The app can't write to its own folder ({folder}), so it can't replace itself. Move wrtier.exe to a folder you can write to, such as your Documents, and try again.")
    })?;
    let _ = std::fs::remove_file(&probe);
    Ok(exe)
}

/// Puts the new exe in place of the running one, which carries on until the app quits.
#[cfg(windows)]
pub fn install(bytes: &[u8], _version: &str) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let new = staged(&exe);
    std::fs::write(&new, bytes).map_err(|e| format!("Couldn't save the new version next to the app: {e}"))?;
    let swapped = self_replace::self_replace(&new);
    let _ = std::fs::remove_file(&new);
    swapped.map_err(|e| format!("Couldn't put the new version in place: {e}"))
}

/// Only a copy installed from the `.deb` can update itself: apt owns its files.
#[cfg(target_os = "linux")]
pub fn ready() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let owned = std::process::Command::new("dpkg-query").arg("-S").arg(&exe).output();
    if !owned.is_ok_and(|o| o.status.success()) {
        return Err("This copy of wrtier wasn't installed from the .deb, so it can't update itself. Install the .deb from the release page instead.".into());
    }
    Ok(exe)
}

/// Installs the new `.deb` with apt, behind the system's password prompt. The running app
/// carries on from the old files until it quits.
#[cfg(target_os = "linux")]
pub fn install(bytes: &[u8], version: &str) -> Result<(), String> {
    let dir = std::env::temp_dir().join(format!("wrtier-update-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let deb = dir.join(format!("wrtier_{version}_amd64.deb"));
    std::fs::write(&deb, bytes).map_err(|e| format!("Couldn't save the download: {e}"))?;
    // apt (unlike dpkg) also installs anything new the package depends on.
    let result = std::process::Command::new("pkexec").args(["apt-get", "install", "-y"]).arg(&deb).output();
    let outcome = match result {
        Err(_) => Err(format!(
            "pkexec isn't installed, so the app can't ask for the password to install. Install it from a terminal: sudo apt install {}",
            deb.display()
        )),
        Ok(out) if out.status.success() => Ok(()),
        // pkexec: 126 when the password prompt is dismissed, 127 when it's refused.
        Ok(out) if matches!(out.status.code(), Some(126 | 127)) => Err("Not installed: the password prompt was cancelled.".into()),
        Ok(out) => {
            let said = String::from_utf8_lossy(&out.stderr);
            let last = said.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
            Err(format!("apt couldn't install the update. {last}"))
        }
    };
    if outcome.is_ok() {
        let _ = std::fs::remove_dir_all(&dir);
    }
    outcome
}

/// Saves the APK for Android's installer. It stays in the app's cache, which the installer
/// reads through the app's FileProvider.
#[cfg(target_os = "android")]
pub fn save_apk(bytes: &[u8], cache: &Path) -> Result<PathBuf, String> {
    let dir = cache.join("update");
    let _ = std::fs::remove_dir_all(&dir); // an older download
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let apk = dir.join("wrtier.apk");
    std::fs::write(&apk, bytes).map_err(|e| format!("Couldn't save the download: {e}"))?;
    Ok(apk)
}

/// What "Check for updates" tells the interface.
#[derive(Serialize, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Checked {
    Newest { current: String },
    Available { current: String, version: String },
    NoBuild { current: String, version: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> semver::Version {
        semver::Version::parse(s).unwrap()
    }

    const MANIFEST: &str = r#"{
        "version": "0.2.0",
        "notes": "",
        "pub_date": "2026-11-11T12:00:00Z",
        "platforms": {
            "windows-x86_64": { "url": "https://example.com/wrtier_0.2.0_windows-x64-portable.exe", "signature": "c2ln" },
            "linux-x86_64": { "url": "https://example.com/wrtier_0.2.0_amd64.deb", "signature": "c2ln" }
        }
    }"#;

    #[test]
    fn a_newer_stable_version_is_offered_with_this_platforms_build() {
        let found = pick(MANIFEST, &v("0.1.2"), "linux-x86_64").unwrap();
        assert_eq!(
            found,
            Found::Available {
                version: "0.2.0".into(),
                build: Build { url: "https://example.com/wrtier_0.2.0_amd64.deb".into(), signature: "c2ln".into() },
            }
        );
    }

    #[test]
    fn the_same_or_an_older_version_is_not_an_update() {
        assert_eq!(pick(MANIFEST, &v("0.2.0"), "linux-x86_64").unwrap(), Found::Newest);
        assert_eq!(pick(MANIFEST, &v("0.3.0"), "linux-x86_64").unwrap(), Found::Newest);
    }

    #[test]
    fn a_pre_release_is_never_offered() {
        let manifest = MANIFEST.replace("\"0.2.0\"", "\"0.2.0-rc1\"");
        assert_eq!(pick(&manifest, &v("0.1.2"), "linux-x86_64").unwrap(), Found::Newest);
    }

    #[test]
    fn a_pre_release_is_updated_to_its_stable_version() {
        assert!(matches!(pick(MANIFEST, &v("0.2.0-rc1"), "windows-x86_64").unwrap(), Found::Available { .. }));
    }

    #[test]
    fn a_version_tag_with_a_v_is_read() {
        let manifest = MANIFEST.replace("\"0.2.0\"", "\"v0.2.0\"");
        assert!(matches!(pick(&manifest, &v("0.1.2"), "windows-x86_64").unwrap(), Found::Available { .. }));
    }

    #[test]
    fn a_release_without_this_platforms_build_says_so() {
        assert_eq!(pick(MANIFEST, &v("0.1.2"), "android-aarch64").unwrap(), Found::NoBuild { version: "0.2.0".into() });
    }

    #[test]
    fn an_unreadable_manifest_is_an_error() {
        assert!(pick("<html>", &v("0.1.2"), "linux-x86_64").is_err());
        assert!(pick(r#"{"version": "soon"}"#, &v("0.1.2"), "linux-x86_64").is_err());
    }

    // Made with the real update key: `tauri signer sign --app-version 0.2.0` of a file holding
    // "wrtier 0.2.0\n", as the release workflow signs each build.
    const SIGNED: &[u8] = b"wrtier 0.2.0\n";
    const SIGNATURE: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVTZFlINXpDdmpjWVdXNmpEQ2EyRmUrZjBCYWVRYmxXcXMzV0hnOXZmbzNab2pTSEZLT0JRTFp3WHpNOG0rVlRSZ2FHMzA4SDNTZExQdW1uVnFvT2ZrUTBHK1pNMHQwS2dJPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzk0Mzk2NTY4CWZpbGU6cGF5bG9hZC50eHQJdmVyc2lvbjowLjIuMApsNWVKQlQwMzd3ZXVCR2JvanJrR25mWm9MMWwrR3Rzd3hiZXNXVGtPRkY5bFVDTWM3b09KN2pMWmlWWnFMWjNoV1JRbHEvcW4vTmFnUlZMT1ZudUlBZz09Cg==";
    // The same file signed without `--app-version`: genuine, but not bound to any version.
    const UNBOUND: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVTZFlINXpDdmpjWWZwVnhpakszSVFuaExuS2M1aWthbTY3dnN6eWM4ZTdlSWNQejVnb0NudTRKeWtNMTNxbDZvdlNhMnBJRGZMWS9ZcldMVGRUVmRnN0pySmZiWFhTY2dZPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzk0Mzk1NjAwCWZpbGU6cGF5bG9hZC50eHQKaDUwUm1nMmVqRkl4NHRyT2JwWlhid2FZTHVMT3g1UG9NcmtOMHQybU56d2U0Y2xjekFhbWVKbzA1SzBTZWNoUGxKRHlnM3hsWFRWb1QxKzdVSCtMQ0E9PQo=";

    #[test]
    fn a_build_signed_with_the_update_key_for_its_version_is_accepted() {
        assert_eq!(verify_with(SIGNED, SIGNATURE, PUBLIC_KEY, "0.2.0"), Ok(()));
    }

    #[test]
    fn a_changed_build_is_refused() {
        assert_eq!(verify_with(b"wrtier 0.2.1\n", SIGNATURE, PUBLIC_KEY, "0.2.0"), Err(BAD_SIGNATURE.into()));
    }

    #[test]
    fn an_older_build_offered_as_a_newer_version_is_refused() {
        assert!(verify_with(SIGNED, SIGNATURE, PUBLIC_KEY, "0.3.0").unwrap_err().contains("version 0.3.0"));
    }

    #[test]
    fn a_build_signed_for_no_version_is_refused() {
        assert!(verify_with(SIGNED, UNBOUND, PUBLIC_KEY, "0.2.0").is_err());
    }

    #[test]
    fn a_missing_or_garbled_signature_is_refused() {
        assert_eq!(verify_with(SIGNED, "", PUBLIC_KEY, "0.2.0"), Err(BAD_SIGNATURE.into()));
        assert_eq!(verify_with(SIGNED, "bm90IGEgc2lnbmF0dXJl", PUBLIC_KEY, "0.2.0"), Err(BAD_SIGNATURE.into()));
    }

    #[test]
    fn a_build_signed_with_another_key_is_refused() {
        // The public half of another key, made with `tauri signer generate` for this test only.
        let other = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEQ2RkIxRTJCRTI5QUU1QzcKUldUSDVacmlLeDc3MW05eU8rRFpzaVJuQ1ZFWFFtcWF0U0xBRmZ3eXlkeUhjRWF5MkxmTGZYSHIK";
        assert_eq!(verify_with(SIGNED, SIGNATURE, other, "0.2.0"), Err(BAD_SIGNATURE.into()));
    }
}
