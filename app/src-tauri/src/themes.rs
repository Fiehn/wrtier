//! Vault themes: CSS files in the Vault's `themes/` folder that change how the app looks.
//!
//! A theme is made outside the app and imported (or put into the folder by hand). Its first line
//! names it and says whether it's light or dark: `/* Theme: Sepia, light */`. The app starts
//! from its own light or dark colours and adds the file on top, so a theme sets only what it
//! changes. The app never writes a theme except by importing one.

use serde::Serialize;
use std::path::{Path, PathBuf};

/// The folder in the Vault.
pub const FOLDER: &str = "themes";

/// A theme in the Vault, as the picker lists it.
#[derive(Serialize, Debug, PartialEq, Eq, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Theme {
    /// Its file's name in `themes/`, which the theme setting holds.
    pub file: String,
    /// The name its header gives.
    pub name: String,
    pub dark: bool,
}

/// A file in `themes/` that isn't a theme, and why.
#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct NotATheme {
    pub file: String,
    pub reason: String,
}

#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct Themes {
    pub themes: Vec<Theme>,
    pub problems: Vec<NotATheme>,
}

/// What a theme's first line must be, for messages.
pub const HEADER_HELP: &str = "A theme's first line must name it and say whether it's light or dark, like /* Theme: Sepia, light */";

/// The name and light/dark from a theme's first line.
pub fn header(text: &str) -> Result<(String, bool), String> {
    let first = text.trim_start_matches('\u{feff}').lines().next().unwrap_or("").trim();
    let inner = first
        .strip_prefix("/*")
        .and_then(|rest| rest.strip_suffix("*/"))
        .map(str::trim)
        .ok_or_else(|| HEADER_HELP.to_string())?;
    let (label, rest) = inner.split_once(':').ok_or_else(|| HEADER_HELP.to_string())?;
    if !label.trim().eq_ignore_ascii_case("theme") {
        return Err(HEADER_HELP.into());
    }
    let (name, kind) = rest.rsplit_once(',').ok_or_else(|| HEADER_HELP.to_string())?;
    let name = name.trim();
    let dark = match kind.trim().to_ascii_lowercase().as_str() {
        "light" => false,
        "dark" => true,
        _ => return Err(HEADER_HELP.into()),
    };
    if name.is_empty() || name.chars().count() > 60 {
        return Err("A theme's name must be 1 to 60 characters.".into());
    }
    Ok((name.to_string(), dark))
}

/// Is `file` a plain file name in `themes/` (no folders, ends in `.css`)?
pub fn is_theme_file(file: &str) -> bool {
    file.to_ascii_lowercase().ends_with(".css")
        && !file.contains(['/', '\\'])
        && file != ".css"
        && !file.starts_with('.')
}

/// The CSS files in `themes/`, sorted. Syncthing's Conflict copies are left out: the Author
/// sorts those out in whatever edits the themes.
fn files(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root.join(FOLDER)) else { return Vec::new() };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            is_theme_file(name) && !name.contains(".sync-conflict-")
        })
        .collect();
    found.sort();
    found
}

/// The Vault's themes, by name, and the files in `themes/` that aren't themes.
pub fn list(root: &Path) -> Themes {
    let mut themes = Vec::new();
    let mut problems = Vec::new();
    for path in files(root) {
        let file = path.file_name().unwrap().to_string_lossy().to_string();
        match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|text| header(&text)) {
            Ok((name, dark)) => themes.push(Theme { file, name, dark }),
            Err(reason) => problems.push(NotATheme { file, reason }),
        }
    }
    themes.sort_by_key(|t| t.name.to_lowercase());
    Themes { themes, problems }
}

/// A theme's CSS.
pub fn read(root: &Path, file: &str) -> Result<String, String> {
    if !is_theme_file(file) {
        return Err(format!("Not a theme: {file:?}"));
    }
    std::fs::read_to_string(root.join(FOLDER).join(file)).map_err(|e| e.to_string())
}

/// What the theme files are now, cheaply: if it changes, a theme was added, changed or removed.
pub fn stamp(root: &Path) -> Vec<(PathBuf, u64, Option<std::time::SystemTime>)> {
    files(root)
        .into_iter()
        .map(|path| {
            let meta = std::fs::metadata(&path).ok();
            let (len, modified) = meta.map(|m| (m.len(), m.modified().ok())).unwrap_or((0, None));
            (path, len, modified)
        })
        .collect()
}

#[derive(Serialize, Debug, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum Imported {
    /// Copied in; the theme setting can now name `file`.
    Imported { file: String, name: String },
    /// The Vault already has a theme by this name: nothing was written. Importing again with
    /// `replace` writes over it.
    Exists { name: String },
}

/// Copies the theme at `source` into the Vault's `themes/`. A file without the header line is
/// refused. A theme with the same name is only replaced when `replace` is set, and keeps its
/// file name, so a setting naming it still finds it.
pub fn import(root: &Path, source: &Path, replace: bool) -> Result<Imported, String> {
    let text = std::fs::read_to_string(source).map_err(|e| format!("{} couldn't be read: {e}", source.display()))?;
    let (name, _) = header(&text)?;
    let existing = list(root).themes.into_iter().find(|t| t.name.eq_ignore_ascii_case(&name));
    let file = match existing {
        Some(_) if !replace => return Ok(Imported::Exists { name }),
        Some(theme) => theme.file,
        None => free_file_name(root, source),
    };
    let folder = root.join(FOLDER);
    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    std::fs::write(folder.join(&file), text).map_err(|e| e.to_string())?;
    Ok(Imported::Imported { file, name })
}

/// The source's file name, or "name (2).css" and so on if another theme has that file name.
fn free_file_name(root: &Path, source: &Path) -> String {
    let stem = source.file_stem().and_then(|s| s.to_str()).filter(|s| !s.is_empty() && !s.starts_with('.')).unwrap_or("theme");
    let folder = root.join(FOLDER);
    let mut file = format!("{stem}.css");
    let mut n = 2;
    while folder.join(&file).exists() {
        file = format!("{stem} ({n}).css");
        n += 1;
    }
    file
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn reads_the_header_line() {
        assert_eq!(header("/* Theme: Sepia, light */\n:root {}"), Ok(("Sepia".into(), false)));
        assert_eq!(header("\u{feff}/*theme:Night Ink,DARK*/"), Ok(("Night Ink".into(), true)));
        // A comma in the name: the last one separates light or dark.
        assert_eq!(header("/* Theme: Paper, ink and dust, dark */"), Ok(("Paper, ink and dust".into(), true)));
        for bad in ["", ":root {}", "/* Sepia, light */", "/* Theme: Sepia */", "/* Theme: Sepia, grey */", "/* Theme: , light */", "// Theme: Sepia, light"] {
            assert!(header(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn lists_themes_by_name_with_the_files_that_arent() {
        let dir = vault();
        let themes = dir.path().join(FOLDER);
        std::fs::create_dir_all(&themes).unwrap();
        std::fs::write(themes.join("b.css"), "/* Theme: Amber, dark */").unwrap();
        std::fs::write(themes.join("a.css"), "/* Theme: Sepia, light */").unwrap();
        std::fs::write(themes.join("notes.css"), ":root { --bg: red; }").unwrap();
        std::fs::write(themes.join("a.sync-conflict-20261109-120000-ABCDEFG.css"), "/* Theme: Sepia, light */").unwrap();
        std::fs::write(themes.join("readme.txt"), "not CSS").unwrap();
        let found = list(dir.path());
        let names: Vec<_> = found.themes.iter().map(|t| (t.name.as_str(), t.file.as_str(), t.dark)).collect();
        assert_eq!(names, [("Amber", "b.css", true), ("Sepia", "a.css", false)]);
        assert_eq!(found.problems.len(), 1);
        assert_eq!(found.problems[0].file, "notes.css");
    }

    #[test]
    fn a_vault_without_themes_has_none() {
        assert_eq!(list(vault().path()), Themes { themes: vec![], problems: vec![] });
    }

    #[test]
    fn imports_and_asks_before_replacing() {
        let dir = vault();
        let source = tempfile::tempdir().unwrap();
        let sepia = source.path().join("sepia.css");
        std::fs::write(&sepia, "/* Theme: Sepia, light */\n:root { --bg: #f4ecd8; }").unwrap();
        assert_eq!(import(dir.path(), &sepia, false), Ok(Imported::Imported { file: "sepia.css".into(), name: "Sepia".into() }));
        assert_eq!(read(dir.path(), "sepia.css").unwrap(), "/* Theme: Sepia, light */\n:root { --bg: #f4ecd8; }");

        // The same name again, from a file called something else: asks, then replaces in place.
        let newer = source.path().join("sepia-2.css");
        std::fs::write(&newer, "/* Theme: sepia, light */\n:root { --bg: #efe4cc; }").unwrap();
        assert_eq!(import(dir.path(), &newer, false), Ok(Imported::Exists { name: "sepia".into() }));
        assert!(read(dir.path(), "sepia.css").unwrap().contains("#f4ecd8"), "nothing written before asking");
        assert_eq!(import(dir.path(), &newer, true), Ok(Imported::Imported { file: "sepia.css".into(), name: "sepia".into() }));
        assert!(read(dir.path(), "sepia.css").unwrap().contains("#efe4cc"));

        // A different theme whose file has a name already taken gets a free one.
        let other = source.path().join("other").join("sepia.css");
        std::fs::create_dir_all(other.parent().unwrap()).unwrap();
        std::fs::write(&other, "/* Theme: Night, dark */").unwrap();
        assert_eq!(import(dir.path(), &other, false), Ok(Imported::Imported { file: "sepia (2).css".into(), name: "Night".into() }));
    }

    #[test]
    fn refuses_a_file_without_the_header() {
        let dir = vault();
        let source = tempfile::tempdir().unwrap();
        let plain = source.path().join("plain.css");
        std::fs::write(&plain, ":root { --bg: red; }").unwrap();
        assert_eq!(import(dir.path(), &plain, false), Err(HEADER_HELP.to_string()));
        assert!(!dir.path().join(FOLDER).exists(), "nothing written");
    }

    #[test]
    fn reads_only_plain_theme_files() {
        let dir = vault();
        for bad in ["../vault.db", "a/b.css", "a\\b.css", "x.txt", ".css"] {
            assert!(read(dir.path(), bad).is_err(), "{bad:?}");
        }
    }
}
