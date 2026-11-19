//! A file's stamp: when it was last modified, and its size. If either changes, the file was
//! written (by the app, another program or Syncthing) since the stamp was taken, which is how
//! the app notices changes without reading whole files.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// When a file was last modified, and how big it is.
pub type Stamp = (SystemTime, u64);

/// `path`'s stamp; None if there's no such file (or it can't be looked at right now).
pub fn file_stamp(path: &Path) -> Option<Stamp> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/// Each of `paths` with its stamp: what a folder of files (the Style library, the themes) is
/// compared by, to notice when any of them was added, changed or removed.
pub fn file_stamps(paths: Vec<PathBuf>) -> Vec<(PathBuf, Option<Stamp>)> {
    paths.into_iter().map(|path| {
        let stamp = file_stamp(&path);
        (path, stamp)
    }).collect()
}
