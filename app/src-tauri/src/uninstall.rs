//! Uninstalling (Windows only: the portable exe has no system uninstaller). Removes the app and
//! what this Device keeps for it (its Device state, recovery files and the editor's cache), and
//! never anything in the Vault. Reinstalling makes a new Device.
//!
//! A running exe, and the WebView's files while it's open, can't be deleted. So the app starts a
//! hidden PowerShell that waits for it to quit, then deletes them, and quits.

#![cfg_attr(not(windows), allow(dead_code))]

use std::path::{Path, PathBuf};

/// The folders the app keeps this Device's data in. Only folders named after the app's
/// identifier count, so a wrong path can never take a whole system folder with it.
pub fn ours(candidates: impl IntoIterator<Item = PathBuf>, identifier: &str) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = candidates
        .into_iter()
        .filter(|d| d.is_absolute() && d.file_name().is_some_and(|n| n == identifier))
        .collect();
    dirs.sort();
    dirs.dedup();
    dirs
}

/// A path as a PowerShell literal: single quotes, with any inside doubled.
fn literal(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "''"))
}

/// The helper's script: wait for the app (process `pid`) to quit, then delete `paths`. The
/// WebView's own processes can hold its files a moment longer, so it tries for about 15 seconds.
pub fn script(pid: u32, paths: &[PathBuf]) -> String {
    let list = paths.iter().map(|p| literal(p)).collect::<Vec<_>>().join(", ");
    format!(
        "$ErrorActionPreference = 'SilentlyContinue'; $ProgressPreference = 'SilentlyContinue'\n\
         Wait-Process -Id {pid} -Timeout 60\n\
         $paths = @({list})\n\
         foreach ($try in 1..30) {{\n\
         \x20 foreach ($p in $paths) {{ Remove-Item -LiteralPath $p -Recurse -Force }}\n\
         \x20 if (-not ($paths | Where-Object {{ Test-Path -LiteralPath $_ }})) {{ break }}\n\
         \x20 Start-Sleep -Milliseconds 500\n\
         }}\n"
    )
}

/// Whether the exe's folder lets the app delete it, checked before anything is deleted.
pub fn ready(exe: &Path) -> Result<(), String> {
    let probe = exe.with_extension("uninstall-check");
    std::fs::write(&probe, b"").map_err(|_| {
        let folder = exe.parent().map(|p| p.display().to_string()).unwrap_or_default();
        format!("The app can't change its own folder ({folder}), so it can't delete itself. Delete wrtier's exe from there by hand.")
    })?;
    let _ = std::fs::remove_file(&probe);
    Ok(())
}

/// The hidden helper that deletes `paths` once process `pid` has quit.
#[cfg(windows)]
fn helper(pid: u32, paths: &[PathBuf]) -> std::process::Command {
    use base64::Engine;
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    // -EncodedCommand takes the script as base64 of UTF-16, so no path needs quoting for the
    // command line.
    let utf16: Vec<u8> = script(pid, paths).encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    let encoded = base64::engine::general_purpose::STANDARD.encode(utf16);
    // By its full path, so a powershell.exe elsewhere on the PATH is never the one run.
    let root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let mut command = std::process::Command::new(root.join(r"System32\WindowsPowerShell\v1.0\powershell.exe"));
    command
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-WindowStyle", "Hidden", "-EncodedCommand", &encoded])
        .creation_flags(CREATE_NO_WINDOW);
    command
}

/// Starts the helper that deletes `paths` once this app has quit.
#[cfg(windows)]
pub fn start(paths: &[PathBuf]) -> Result<(), String> {
    helper(std::process::id(), paths)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Couldn't start removing the app: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_folders_named_after_the_app_are_removed() {
        let local = PathBuf::from(r"C:\Users\A\AppData\Local\com.fiehn.wrtier");
        let found = ours(
            [
                local.clone(),
                PathBuf::from(r"C:\Users\A\AppData\Local"),                // the system's folder
                PathBuf::from(r"C:\Users\A\AppData\Local\com.fiehn.wrtier\logs"), // inside the first
                PathBuf::from("com.fiehn.wrtier"),                          // relative: never
                local.clone(),
            ],
            "com.fiehn.wrtier",
        );
        assert_eq!(found, vec![local]);
    }

    #[test]
    fn the_script_waits_for_the_app_and_quotes_every_path() {
        let script = script(4242, &[PathBuf::from(r"C:\Users\O'Brien\AppData\Local\com.fiehn.wrtier"), PathBuf::from(r"D:\Apps\wrtier.exe")]);
        assert!(script.contains("Wait-Process -Id 4242"));
        assert!(script.contains(r"'C:\Users\O''Brien\AppData\Local\com.fiehn.wrtier', 'D:\Apps\wrtier.exe'"));
        assert!(script.find("Wait-Process").unwrap() < script.find("Remove-Item").unwrap(), "nothing goes before the app has quit");
    }

    #[test]
    fn a_folder_the_app_can_write_to_is_ready() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(ready(&dir.path().join("wrtier.exe")), Ok(()));
        assert!(std::fs::read_dir(dir.path()).unwrap().next().is_none(), "the check leaves nothing behind");
    }

    #[cfg(windows)]
    #[test]
    fn the_helper_deletes_the_folders_and_the_exe_once_the_app_has_quit() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("O'Brien data").join("com.fiehn.wrtier");
        std::fs::create_dir_all(data.join("EBWebView").join("Default")).unwrap();
        std::fs::write(data.join("device.json"), "{}").unwrap();
        let exe = dir.path().join("wrtier.exe");
        std::fs::write(&exe, b"MZ").unwrap();
        let other = dir.path().join("Novels.db"); // anything else in the folder stays
        std::fs::write(&other, b"").unwrap();

        // An "app" that has already quit: the helper goes straight on to deleting.
        let mut quit = std::process::Command::new("cmd").args(["/c", "exit"]).spawn().unwrap();
        let pid = quit.id();
        quit.wait().unwrap();
        assert!(helper(pid, &[data.clone(), exe.clone()]).status().unwrap().success());

        assert!(!data.exists(), "the app's data is gone");
        assert!(!exe.exists(), "the exe is gone");
        assert!(other.exists(), "nothing else is touched");
    }

    #[test]
    fn a_folder_that_doesnt_exist_isnt_ready() {
        let dir = tempfile::tempdir().unwrap();
        assert!(ready(&dir.path().join("gone").join("wrtier.exe")).is_err());
    }
}
