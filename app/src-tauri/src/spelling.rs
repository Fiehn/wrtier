//! The Spellchecker's Dictionaries (CONTEXT.md, ADR 0019). The interface reads the text and
//! collects its words; this side only says which words a Dictionary doesn't know, and what it
//! suggests for one.
//!
//! A Work is checked in its language and region ("en-GB"). The Dictionary for it is looked for
//! in the Vault's `dictionaries/<code>/` first (`index.aff`, `index.dic`, the `license` beside
//! them), then among the two that ship with the app (British and American English); a
//! Device's own dictionaries are never used, so every Device checks a Work alike. Without a
//! region, or with one there's no Dictionary for, the language's default is used (British for
//! English, otherwise the Dictionary named by the language alone), and the interface says so.
//!
//! The Dictionaries the Author can download are listed in `dictionaries/catalogue.json`, written
//! by `scripts/dictionaries.mjs`: every dictionary in wooorm/dictionaries at one pinned commit,
//! with the SHA-256 of each file. The app never asks GitHub what's there, and keeps nothing it
//! downloaded that doesn't match.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use spellbook::Dictionary as Hunspell;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime};

/// The Vault's folder of Dictionaries.
pub const FOLDER: &str = "dictionaries";
/// What a downloaded Dictionary was downloaded as: the commit and its files' hashes, so a newer
/// App version knows whether its own list has a newer one. A Dictionary put in the folder by
/// hand has none, and is never offered an update.
const SOURCE: &str = "source.json";
/// The default for a language with no region, where it isn't the Dictionary named by the
/// language alone.
const DEFAULTS: &[(&str, &str)] = &[("en", "en-GB")];
/// Far bigger than any dictionary in the list (the biggest is about 25 MB).
const MAX_FILE: u64 = 64 * 1024 * 1024;

/// The two that ship inside the app.
const BUILT_IN: &[(&str, &str, &str)] = &[
    ("en-GB", include_str!("../dictionaries/en-GB/index.aff"), include_str!("../dictionaries/en-GB/index.dic")),
    ("en-US", include_str!("../dictionaries/en-US/index.aff"), include_str!("../dictionaries/en-US/index.dic")),
];

/// A Dictionary the app offers to download.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Offered {
    /// Ours: the language, then the region in capitals ("en-GB", "de-CH", "da").
    pub code: String,
    /// Its folder in wooorm/dictionaries.
    #[serde(skip_serializing)]
    pub source: String,
    /// "English (United Kingdom)".
    pub name: String,
    /// As an SPDX expression: "(MIT AND BSD)", "GPL-2.0 OR LGPL-2.1 OR MPL-1.1".
    pub license: String,
    /// Bytes, both files.
    pub size: u64,
    #[serde(skip_serializing)]
    pub aff: String,
    #[serde(skip_serializing)]
    pub dic: String,
}

#[derive(Deserialize)]
struct Catalogue {
    repo: String,
    commit: String,
    dictionaries: Vec<Offered>,
}

fn catalogue() -> &'static Catalogue {
    static CATALOGUE: OnceLock<Catalogue> = OnceLock::new();
    CATALOGUE.get_or_init(|| serde_json::from_str(include_str!("../dictionaries/catalogue.json")).expect("the catalogue is written by scripts/dictionaries.mjs"))
}

fn offered(code: &str) -> Option<&'static Offered> {
    catalogue().dictionaries.iter().find(|d| d.code == code)
}

/// What a Dictionary is called: its name in the list, or its code if it isn't there.
fn name_of(code: &str) -> String {
    offered(code).map(|d| d.name.clone()).unwrap_or_else(|| code.to_string())
}

/// "en-gb" → ("en", Some("GB")); "" → ("en", None), Typst's default. Nothing else is checked here:
/// the Preamble only lets codes through.
fn split(language: &str) -> (String, Option<String>) {
    let language = language.trim();
    let (lang, region) = match language.split_once('-') {
        Some((l, r)) => (l, Some(r.to_ascii_uppercase())),
        None => (language, None),
    };
    let lang = if lang.is_empty() { "en".to_string() } else { lang.to_ascii_lowercase() };
    (lang, region.filter(|r| !r.is_empty()))
}

/// Is `code` something a Dictionary's folder may be called?
fn is_code(code: &str) -> bool {
    !code.is_empty() && code.len() <= 20 && code.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') && !code.starts_with('-')
}

/// Where a Dictionary's files are.
#[derive(Clone, Debug, PartialEq)]
enum Source {
    Vault(PathBuf),
    BuiltIn(usize),
}

fn find(root: &Path, code: &str) -> Option<Source> {
    let dir = root.join(FOLDER).join(code);
    if dir.join("index.aff").is_file() && dir.join("index.dic").is_file() {
        return Some(Source::Vault(dir));
    }
    BUILT_IN.iter().position(|(c, _, _)| *c == code).map(Source::BuiltIn)
}

/// The Dictionary a Work in `language` is checked with: its code, where it is, and the code
/// asked for when it's a stand-in for one there's no Dictionary for.
fn resolve(root: &Path, language: &str) -> Option<(String, Source, Option<String>)> {
    let (lang, region) = split(language);
    let asked = region.as_ref().map(|r| format!("{lang}-{r}"));
    if let Some(code) = &asked
        && let Some(source) = find(root, code)
    {
        return Some((code.clone(), source, None));
    }
    let default = DEFAULTS.iter().find(|(l, _)| *l == lang).map(|(_, d)| d.to_string()).unwrap_or(lang);
    let source = find(root, &default)?;
    Some((default, source, asked))
}

/// What the Mods dialog says about a Work's Dictionary.
#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DictionaryFor {
    /// The Dictionary used, if any: its code and name, and whether it ships with the app.
    pub code: Option<String>,
    pub name: Option<String>,
    pub built_in: bool,
    /// The language and region asked for (its name), when the one used stands in for it.
    pub instead_of: Option<String>,
    /// The Dictionary for exactly what was asked, if the app offers to download it.
    pub offer: Option<Offered>,
}

pub fn dictionary_for(root: &Path, language: &str) -> DictionaryFor {
    let (lang, region) = split(language);
    let exact = match &region {
        Some(r) => format!("{lang}-{r}"),
        None => DEFAULTS.iter().find(|(l, _)| *l == lang).map(|(_, d)| d.to_string()).unwrap_or(lang.clone()),
    };
    let offer = offered(&exact).filter(|_| find(root, &exact).is_none()).cloned()
        // No region and no Dictionary named by the language alone: the first for that language.
        .or_else(|| if region.is_none() && find(root, &exact).is_none() {
            catalogue().dictionaries.iter().find(|d| split(&d.code).0 == lang).cloned()
        } else { None });
    match resolve(root, language) {
        Some((code, source, asked)) => DictionaryFor {
            name: Some(name_of(&code)),
            built_in: matches!(source, Source::BuiltIn(_)),
            code: Some(code),
            instead_of: asked.map(|a| name_of(&a)),
            offer,
        },
        None => DictionaryFor { code: None, name: None, built_in: false, instead_of: None, offer },
    }
}

/// When a file in the Vault was last changed, and its length: a Dictionary is read again when
/// either of its files changes (another Device downloaded a newer one).
type Stamp = Vec<(u64, Option<SystemTime>)>;

fn stamp(source: &Source) -> Stamp {
    match source {
        Source::BuiltIn(_) => Vec::new(),
        Source::Vault(dir) => ["index.aff", "index.dic"].iter()
            .map(|f| std::fs::metadata(dir.join(f)).map(|m| (m.len(), m.modified().ok())).unwrap_or((0, None)))
            .collect(),
    }
}

/// The Dictionaries read so far, by where they are. Reading one takes a moment (a large one
/// about half a second), so each is read once and kept.
#[derive(Default)]
pub struct Spelling {
    loaded: HashMap<String, (Source, Stamp, Arc<Hunspell>)>,
}

impl Spelling {
    /// The Dictionary for `language`, read if it isn't yet. None if there's none for it.
    fn dictionary(&mut self, root: &Path, language: &str) -> Result<Option<Arc<Hunspell>>, String> {
        let Some((code, source, _)) = resolve(root, language) else { return Ok(None) };
        let now = stamp(&source);
        if let Some((s, st, dict)) = self.loaded.get(&code)
            && *s == source
            && *st == now
        {
            return Ok(Some(dict.clone()));
        }
        let dict = match &source {
            Source::BuiltIn(i) => Hunspell::new(BUILT_IN[*i].1, BUILT_IN[*i].2),
            Source::Vault(dir) => {
                let read = |f: &str| std::fs::read(dir.join(f)).map(|b| String::from_utf8_lossy(&b).into_owned())
                    .map_err(|e| format!("Couldn't read the {} Dictionary: {e}", name_of(&code)));
                Hunspell::new(&read("index.aff")?, &read("index.dic")?)
            }
        }
        .map_err(|e| format!("The {} Dictionary can't be read: {e}", name_of(&code)))?;
        let dict = Arc::new(dict);
        self.loaded.insert(code, (source, now, dict.clone()));
        Ok(Some(dict))
    }

    /// Which of `words` the Dictionary for `language` doesn't know. None if there's no
    /// Dictionary for it (nothing is marked).
    pub fn check(&mut self, root: &Path, language: &str, words: &[String]) -> Result<Option<Vec<String>>, String> {
        let Some(dict) = self.dictionary(root, language)? else { return Ok(None) };
        Ok(Some(words.iter().filter(|w| !dict.check(w)).cloned().collect()))
    }

    /// What the Dictionary suggests for `word`, best first.
    pub fn suggest(&mut self, root: &Path, language: &str, word: &str) -> Result<Vec<String>, String> {
        let Some(dict) = self.dictionary(root, language)? else { return Ok(Vec::new()) };
        let mut out = Vec::new();
        dict.suggest(word, &mut out);
        Ok(out)
    }

    /// Forgets the Dictionaries read from the Vault (one was downloaded or removed).
    pub fn forget(&mut self) {
        self.loaded.retain(|_, (s, _, _)| matches!(s, Source::BuiltIn(_)));
    }
}

// ---- Downloading, listing and removing ----

/// A Dictionary in the Vault.
#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Kept {
    pub code: String,
    pub name: String,
    /// From the list, if it came from there.
    pub license: Option<String>,
    /// The app's list has a newer version of the one downloaded.
    pub update: bool,
}

/// Settings' list: the Dictionaries in the Vault, those built in, and those the app offers.
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Dictionaries {
    pub in_vault: Vec<Kept>,
    pub built_in: Vec<Kept>,
    pub offered: Vec<Offered>,
}

#[derive(Serialize, Deserialize)]
struct Downloaded {
    commit: String,
    aff: String,
    dic: String,
}

pub fn list(root: &Path) -> Dictionaries {
    let mut in_vault: Vec<Kept> = std::fs::read_dir(root.join(FOLDER)).into_iter().flatten().flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|code| is_code(code) && matches!(find(root, code), Some(Source::Vault(_))))
        .map(|code| {
            let source: Option<Downloaded> = std::fs::read_to_string(root.join(FOLDER).join(&code).join(SOURCE)).ok()
                .and_then(|t| serde_json::from_str(&t).ok());
            let offer = offered(&code);
            Kept {
                name: name_of(&code),
                license: offer.map(|o| o.license.clone()),
                update: matches!((&source, offer), (Some(s), Some(o)) if s.aff != o.aff || s.dic != o.dic),
                code,
            }
        })
        .collect();
    in_vault.sort_by(|a, b| a.name.cmp(&b.name));
    let built_in = BUILT_IN.iter()
        .map(|(code, _, _)| Kept { code: code.to_string(), name: name_of(code), license: offered(code).map(|o| o.license.clone()), update: false })
        .collect();
    let mut offered: Vec<Offered> = catalogue().dictionaries.iter()
        .filter(|d| !in_vault.iter().any(|k| k.code == d.code))
        .cloned()
        .collect();
    offered.sort_by(|a, b| a.name.cmp(&b.name));
    Dictionaries { in_vault, built_in, offered }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .user_agent("wrtier-dictionaries")
        .build()
        .into()
}

fn fetch(url: &str) -> Result<Vec<u8>, String> {
    let response = agent().get(url).call().map_err(|e| match e {
        ureq::Error::HostNotFound | ureq::Error::ConnectionFailed | ureq::Error::Timeout(_) => {
            "Couldn't reach GitHub. Check the internet connection and try again.".to_string()
        }
        e => format!("Couldn't download it from GitHub: {e}"),
    })?;
    let mut bytes = Vec::new();
    response.into_body().into_with_config().limit(MAX_FILE).reader().read_to_end(&mut bytes)
        .map_err(|e| format!("The download stopped: {e}"))?;
    Ok(bytes)
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Downloads a Dictionary the app offers into the Vault (or a newer version over the one there),
/// checked against the list before anything is written.
pub fn download(root: &Path, code: &str) -> Result<(), String> {
    let catalogue = catalogue();
    let offer = offered(code).ok_or_else(|| format!("The app doesn't offer a Dictionary called {code}."))?;
    let url = |file: &str| format!("https://raw.githubusercontent.com/{}/{}/dictionaries/{}/{file}", catalogue.repo, catalogue.commit, offer.source);
    let aff = fetch(&url("index.aff"))?;
    let dic = fetch(&url("index.dic"))?;
    if sha256(&aff) != offer.aff || sha256(&dic) != offer.dic {
        return Err(format!("The {} Dictionary downloaded isn't the one the app expects, so it wasn't kept.", offer.name));
    }
    let license = fetch(&url("license")).unwrap_or_default();
    let source = serde_json::to_vec_pretty(&Downloaded { commit: catalogue.commit.clone(), aff: offer.aff.clone(), dic: offer.dic.clone() })
        .map_err(|e| e.to_string())?;
    let dir = root.join(FOLDER).join(code);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Couldn't make the folder for it: {e}"))?;
    // Each file goes in whole: written beside it, then renamed over it.
    for (name, bytes) in [("license", &license), ("index.aff", &aff), ("index.dic", &dic), (SOURCE, &source)] {
        let part = dir.join(format!("{name}.part"));
        std::fs::write(&part, bytes).and_then(|()| std::fs::rename(&part, dir.join(name)))
            .map_err(|e| format!("Couldn't save the {} Dictionary: {e}", offer.name))?;
    }
    Ok(())
}

/// Removes a Dictionary from the Vault (for every Device). One that ships with the app stays.
pub fn remove(root: &Path, code: &str) -> Result<(), String> {
    if !is_code(code) {
        return Err(format!("Not a Dictionary: {code:?}"));
    }
    let dir = root.join(FOLDER).join(code);
    if !dir.is_dir() {
        return Ok(());
    }
    std::fs::remove_dir_all(&dir).map_err(|e| format!("Couldn't remove the {} Dictionary: {e}", name_of(code)))
}

/// The Conflict copies in the Vault's Dictionaries. A copy identical to its file is deleted
/// without a word (two Devices downloaded the same one before syncing); any other is returned,
/// as the path from the Vault's folder and why, to be reported as a Sync problem.
pub fn conflicts(root: &Path) -> Vec<(String, String)> {
    let mut problems = Vec::new();
    for dir in std::fs::read_dir(root.join(FOLDER)).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()) {
        for copy in std::fs::read_dir(&dir).into_iter().flatten().flatten().map(|e| e.path()) {
            let Some(main) = wrtier::conflicts::main_of(&copy) else { continue };
            let same = matches!((std::fs::read(&copy), std::fs::read(&main)), (Ok(a), Ok(b)) if a == b);
            if same && std::fs::remove_file(&copy).is_ok() {
                continue;
            }
            let relative = copy.strip_prefix(root).unwrap_or(&copy).to_string_lossy().replace('\\', "/");
            let main_name = main.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            problems.push((relative, format!("another version of {main_name}: delete the one you don't want, or download the Dictionary again")));
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_work_is_checked_in_its_language_and_region_with_british_english_the_default() {
        let dir = tempfile::tempdir().unwrap();
        let mut spelling = Spelling::default();
        let both = words(&["colour", "color", "teh"]);
        assert_eq!(spelling.check(dir.path(), "en-GB", &both).unwrap(), Some(words(&["color", "teh"])));
        assert_eq!(spelling.check(dir.path(), "en-US", &both).unwrap(), Some(words(&["colour", "teh"])));
        // No region, or none at all (Typst's default): British.
        assert_eq!(spelling.check(dir.path(), "en", &both).unwrap(), Some(words(&["color", "teh"])));
        assert_eq!(spelling.check(dir.path(), "", &both).unwrap(), Some(words(&["color", "teh"])));
        // A region there's no Dictionary for: the language's default, and it says so.
        let za = dictionary_for(dir.path(), "en-ZA");
        assert_eq!(za.code.as_deref(), Some("en-GB"));
        assert_eq!(za.instead_of.as_deref(), Some("English (South Africa)"));
        assert_eq!(za.offer.map(|o| o.code).as_deref(), Some("en-ZA"));
        // A language there's none for: nothing is marked, and it's offered.
        assert_eq!(spelling.check(dir.path(), "da", &both).unwrap(), None);
        let da = dictionary_for(dir.path(), "da");
        assert_eq!((da.code, da.offer.map(|o| o.code)), (None, Some("da".to_string())));
        assert!(dictionary_for(dir.path(), "en-GB").built_in);
    }

    #[test]
    fn it_suggests_words_the_dictionary_knows() {
        let dir = tempfile::tempdir().unwrap();
        let mut spelling = Spelling::default();
        let suggestions = spelling.suggest(dir.path(), "en-GB", "recieve").unwrap();
        assert!(suggestions.iter().any(|s| s == "receive"), "{suggestions:?}");
    }

    #[test]
    fn a_dictionary_in_the_vault_comes_before_the_built_in_one_and_is_read_again_when_it_changes() {
        let dir = tempfile::tempdir().unwrap();
        let mut spelling = Spelling::default();
        assert_eq!(spelling.check(dir.path(), "en-GB", &words(&["zorp"])).unwrap(), Some(words(&["zorp"])));
        let gb = dir.path().join(FOLDER).join("en-GB");
        std::fs::create_dir_all(&gb).unwrap();
        std::fs::write(gb.join("index.aff"), "SET UTF-8\n").unwrap();
        std::fs::write(gb.join("index.dic"), "1\nzorp\n").unwrap();
        assert_eq!(spelling.check(dir.path(), "en-GB", &words(&["zorp", "colour"])).unwrap(), Some(words(&["colour"])));
        assert!(!dictionary_for(dir.path(), "en-GB").built_in);
        std::fs::write(gb.join("index.dic"), "2\nzorp\ncolour\n").unwrap();
        assert_eq!(spelling.check(dir.path(), "en-GB", &words(&["zorp", "colour"])).unwrap(), Some(Vec::new()));
        // Put there by hand: in the list, never offered an update; removed, the built-in one is back.
        let listed = list(dir.path());
        assert_eq!(listed.in_vault.iter().map(|k| (k.code.as_str(), k.update)).collect::<Vec<_>>(), [("en-GB", false)]);
        assert!(!listed.offered.iter().any(|o| o.code == "en-GB"));
        remove(dir.path(), "en-GB").unwrap();
        assert!(dictionary_for(dir.path(), "en-GB").built_in);
        assert!(remove(dir.path(), "../styles").is_err());
    }

    #[test]
    fn an_identical_conflict_copy_of_a_dictionary_is_deleted_and_any_other_reported() {
        let dir = tempfile::tempdir().unwrap();
        let da = dir.path().join(FOLDER).join("da");
        std::fs::create_dir_all(&da).unwrap();
        std::fs::write(da.join("index.dic"), "1\nhej\n").unwrap();
        std::fs::write(da.join("index.sync-conflict-20261117-101500-ABCDEFG.dic"), "1\nhej\n").unwrap();
        std::fs::write(da.join("index.aff"), "SET UTF-8\n").unwrap();
        std::fs::write(da.join("index.sync-conflict-20261117-101500-ABCDEFG.aff"), "SET ISO8859-1\n").unwrap();
        let problems = conflicts(dir.path());
        assert!(!da.join("index.sync-conflict-20261117-101500-ABCDEFG.dic").exists());
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].0, "dictionaries/da/index.sync-conflict-20261117-101500-ABCDEFG.aff");
    }

    #[test]
    fn the_list_is_pinned_and_every_entry_has_hashes() {
        let c = catalogue();
        assert_eq!(c.commit.len(), 40);
        assert!(c.dictionaries.len() > 50);
        assert!(c.dictionaries.iter().all(|d| d.aff.len() == 64 && d.dic.len() == 64 && is_code(&d.code)));
        assert!(offered("en-US").is_some_and(|d| d.source == "en"));
    }
}
