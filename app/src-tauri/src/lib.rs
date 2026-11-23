//! The Rust side of the app: it holds the History engine, the open Vault and its open Works,
//! and answers the interface. The interface only displays and captures input (ADR 0005).

pub mod colouring;
pub mod device;
pub mod docx;
pub mod folders;
pub mod grammar;
pub mod harper;
pub mod images;
pub mod library;
pub mod patch;
pub mod preamble;
pub mod positions;
pub mod recovery;
pub mod spelling;
pub mod stamps;
pub mod syncthing;
pub mod themes;
pub mod typeset;
pub mod uninstall;
pub mod update;
pub mod vault;
pub mod vaults;
pub mod work;

use device::DeviceState;
use serde::Serialize;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
use tauri_plugin_dialog::DialogExt;
use vault::{Listing, Opened, OpenedWork, Vault};
use work::{Op, WorkState};

struct App {
    device: DeviceState,
    vault: Option<Vault>,
    /// This Device's recovery files, outside every Vault.
    recovery_dir: PathBuf,
}

struct AppState(Mutex<App>);

/// The Page previews. Kept apart from `AppState`: a compile can take a while (the first one of
/// a novel about half a second), and typing must never wait for it. Holding this lock for the
/// whole compile also makes sure only one compile runs at a time (Phase 0b).
struct TypesetState(Mutex<typeset::Typesetter>);

/// The Dictionaries read so far (spelling.rs). Kept apart from `AppState`: reading one takes a
/// moment, and typing must never wait for it.
struct SpellState(Mutex<spelling::Spelling>);

/// The Vault's folder, for the Spellchecker: the app's lock is held only to find it.
fn vault_root(handle: &AppHandle) -> Result<PathBuf, String> {
    with_app(&handle.state::<AppState>(), |app| Ok(open_vault_of(app)?.root.clone()))
}

fn with_spelling<T>(handle: &AppHandle, f: impl FnOnce(&mut spelling::Spelling, &std::path::Path) -> Result<T, String>) -> Result<T, String> {
    let root = vault_root(handle)?;
    guarded(&handle.state::<SpellState>().0, |spelling| f(spelling, &root))
}

/// Which of `words` the Dictionary for `language` doesn't know (ADR 0019); None if there's no
/// Dictionary for it. Off the main thread: the first check reads the Dictionary.
#[tauri::command]
async fn spell_check(handle: AppHandle, language: String, words: Vec<String>) -> Result<Option<Vec<String>>, String> {
    tauri::async_runtime::spawn_blocking(move || with_spelling(&handle, |s, root| s.check(root, &language, &words)))
        .await
        .map_err(|e| e.to_string())?
}

/// What the Dictionary for `language` suggests for `word`, best first.
#[tauri::command]
async fn spell_suggest(handle: AppHandle, language: String, word: String) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || with_spelling(&handle, |s, root| s.suggest(root, &language, &word)))
        .await
        .map_err(|e| e.to_string())?
}

/// Which Dictionary a Work in `language` is checked with, and which could be downloaded for it.
#[tauri::command]
fn dictionary_for(handle: AppHandle, language: String) -> Result<spelling::DictionaryFor, String> {
    Ok(spelling::dictionary_for(&vault_root(&handle)?, &language))
}

/// The Dictionaries in the Vault, those built in, and those the app offers to download.
#[tauri::command]
fn list_dictionaries(handle: AppHandle) -> Result<spelling::Dictionaries, String> {
    Ok(spelling::list(&vault_root(&handle)?))
}

/// Downloads a Dictionary the app offers into the Vault (or its newer version).
#[tauri::command]
async fn download_dictionary(handle: AppHandle, code: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        spelling::download(&vault_root(&handle)?, &code)?;
        with_spelling(&handle, |s, _| Ok(s.forget()))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Removes a Dictionary from the Vault.
#[tauri::command]
fn remove_dictionary(handle: AppHandle, code: String) -> Result<(), String> {
    spelling::remove(&vault_root(&handle)?, &code)?;
    with_spelling(&handle, |s, _| Ok(s.forget()))
}

/// Harper (grammar.rs) on its own thread (harper.rs), loaded by the first question from a Work
/// with the Grammar checker or Syntax highlighting. Kept apart from `AppState`, like the
/// Dictionaries: loading takes half a second, and checking a long Work for the first time much longer.
struct HarperState(harper::HarperThread);

/// What Harper finds in each of `paragraphs`, as the text reads, with the rules in `off` switched
/// off (ADR 0023); None if the Work isn't in English. After any word types asked for.
#[tauri::command]
async fn grammar_check(handle: AppHandle, language: String, paragraphs: Vec<String>, off: Vec<String>) -> Result<Option<Vec<Vec<grammar::Found>>>, String> {
    let answer = handle.state::<HarperState>().0.run(harper::Queue::Later, move |g| g.check(&language, &paragraphs, &off));
    answer.await
}

/// The type of each word in each of `paragraphs`, as the text reads (the Syntax highlighting
/// Mod); None if the Work isn't in English. Before any grammar waiting: it's what's on screen.
#[tauri::command]
async fn word_types(handle: AppHandle, language: String, paragraphs: Vec<String>) -> Result<Option<Vec<Vec<grammar::Word>>>, String> {
    let answer = handle.state::<HarperState>().0.run(harper::Queue::First, move |_| grammar::word_types(&language, &paragraphs));
    answer.await
}

/// The update "Check for updates" last found (its App version and this platform's build), kept
/// for "Download and install".
struct UpdateState(Mutex<Option<(String, update::Build)>>);

/// The time, always from here and never from the interface.
fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// An error, as the interface shows it.
pub(crate) fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Runs `f` on what `lock` holds. A panic that gets this far becomes an error message instead
/// of closing the app, and one that poisoned the lock earlier doesn't stop it: the code inside
/// catches panics where its state could be left half-updated (the Vault, around each Work).
fn guarded<S, T>(lock: &Mutex<S>, f: impl FnOnce(&mut S) -> Result<T, String>) -> Result<T, String> {
    let mut held = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    match catch_unwind(AssertUnwindSafe(|| f(&mut held))) {
        Ok(result) => result,
        Err(panic) => Err(format!("Something went wrong: {}", vault::panic_message(panic.as_ref()))),
    }
}

/// Runs a command on the app's state (see `guarded`).
fn with_app<T>(state: &AppState, f: impl FnOnce(&mut App) -> Result<T, String>) -> Result<T, String> {
    guarded(&state.0, f)
}

/// Runs `f` on the typesetter (see `guarded`).
fn with_typesetter<T>(handle: &AppHandle, f: impl FnOnce(&mut typeset::Typesetter) -> Result<T, String>) -> Result<T, String> {
    guarded(&handle.state::<TypesetState>().0, f)
}

/// `with_app` on a worker thread instead of the main one: while a command runs on the main
/// thread, the window can't repaint or take input. For commands that read whole files or walk
/// the Vault. (Typing's `edit` stays on the main thread: it's quick, and its order with the
/// other editing commands matters. `WorkSync` sends one batch at a time per Work, so moving
/// other commands off it doesn't reorder typing.)
async fn off_main<T: Send + 'static>(handle: AppHandle, f: impl FnOnce(&mut App) -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || with_app(&handle.state::<AppState>(), f))
        .await
        .map_err(|e| e.to_string())?
}

fn open_vault_of(app: &mut App) -> Result<&mut Vault, String> {
    app.vault.as_mut().ok_or_else(|| "No Vault is open.".to_string())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Startup {
    last_vault: Option<String>,
}

#[tauri::command]
fn startup(state: State<AppState>) -> Result<Startup, String> {
    with_app(&state, |app| Ok(Startup { last_vault: app.device.last_vault().map(|p| p.display().to_string()) }))
}

/// On Android the Vault is picked with the app's own folder browser instead (Android's picker
/// gives no file path): see `list_folders`.
#[cfg(mobile)]
#[tauri::command]
async fn pick_folder(_app: AppHandle, _title: Option<String>) -> Result<Option<String>, String> {
    Err("On Android, Vaults are chosen with the app's folder browser.".to_string())
}

/// The folder browser's view of one folder (Android).
#[tauri::command]
fn list_folders(path: String) -> Result<folders::FolderListing, String> {
    folders::list(std::path::Path::new(&path))
}

#[cfg(desktop)]
#[tauri::command]
async fn pick_folder(app: AppHandle, title: Option<String>) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.dialog().file().set_title(title.as_deref().unwrap_or("Choose your Vault")).blocking_pick_folder()
            .and_then(|path| path.into_path().ok())
            .map(|path| path.display().to_string())
    })
    .await
    .map_err(|e| e.to_string())
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
enum VaultOpened {
    /// The folder has no `vault.db`: the interface asks before making it a Vault.
    NotAVault,
    #[serde(rename_all = "camelCase")]
    Opened { root: String, name: String, device_name: Option<String>, suggested_name: String },
}

#[tauri::command]
fn open_vault(state: State<AppState>, path: String, create: bool) -> Result<VaultOpened, String> {
    with_app(&state, |app| {
        let root = PathBuf::from(&path);
        match Vault::open(&root, app.device.device, create, &app.recovery_dir)? {
            Opened::NotAVault => Ok(VaultOpened::NotAVault),
            Opened::Vault(vault) => {
                let device_name = vault.device_name();
                let name = vault.name();
                if let Some(previous) = app.vault.as_mut() {
                    previous.close_all(now_ms());
                }
                app.vault = Some(vault);
                typeset::warm_up();
                // First on this Device's Known Vaults, with its name as it is now.
                app.device.remember_vault(&root, &name);
                app.device.save()?;
                Ok(VaultOpened::Opened { root: path, name, device_name, suggested_name: device::suggested_name() })
            }
        }
    })
}

// ---- Known Vaults and Manage Vaults (v3 plan, Vault management; ADR 0022) ----

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct KnownVaultRow {
    path: String,
    name: String,
    /// Its folder is where the list says (not on a drive that's unplugged, say).
    found: bool,
    /// It's the Vault open now.
    open: bool,
}

/// This Device's Known Vaults, the most recently opened first.
#[tauri::command]
fn known_vaults(state: State<AppState>) -> Result<Vec<KnownVaultRow>, String> {
    with_app(&state, |app| {
        let open = app.vault.as_ref().map(|v| v.root.clone());
        Ok(app.device.known_vaults.iter().map(|k| KnownVaultRow {
            path: k.path.display().to_string(),
            name: k.name.clone(),
            found: k.path.is_dir(),
            open: open.as_deref() == Some(k.path.as_path()),
        }).collect())
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct VaultFolder {
    /// It's a Vault already.
    vault: bool,
    /// How many files are in it (up to 10,000), to say before making it a Vault.
    files: usize,
}

/// A folder to open as a Vault: is it one, and if not, what's in it.
#[tauri::command]
fn vault_folder(path: String) -> Result<VaultFolder, String> {
    let root = PathBuf::from(&path);
    if !root.is_dir() {
        return Err(format!("{path} isn't a folder."));
    }
    let vault = root.join(vault::VAULT_FILE).exists() || !wrtier::conflicts::conflict_copies(&root.join(vault::VAULT_FILE)).is_empty();
    Ok(VaultFolder { vault, files: if vault { 0 } else { vaults::count_files(&root, 10_000) } })
}

/// Makes a new Vault called `name` in a folder of its own in `location`: its folder's path. The
/// interface opens it next.
#[tauri::command]
fn create_vault(state: State<AppState>, name: String, location: String) -> Result<String, String> {
    let name = vault::checked_name(&name)?;
    let folder = vaults::new_vault_folder(std::path::Path::new(&location), &name)?;
    std::fs::create_dir(&folder).map_err(err)?;
    with_app(&state, |app| match Vault::open(&folder, app.device.device, true, &app.recovery_dir)? {
        Opened::Vault(mut made) => made.set_name(&name, now_ms()),
        Opened::NotAVault => Err("The Vault couldn't be made.".into()),
    })?;
    Ok(folder.display().to_string())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SyncthingAt {
    /// Syncthing is set up on this Device (its settings were found, or given by hand).
    found: bool,
    /// It answered.
    running: bool,
    /// How it syncs the folder, if it answered.
    synced: Option<syncthing::Synced>,
}

/// How Syncthing syncs a folder (a Vault, or where a new one would go): for the Syncthing tick in
/// Manage Vaults.
#[tauri::command]
async fn syncthing_at(state: State<'_, AppState>, path: String) -> Result<SyncthingAt, String> {
    let manual = with_app(&state, |app| Ok(app.device.syncthing.clone()))?;
    let Some(connection) = manual.or_else(syncthing::find) else {
        return Ok(SyncthingAt { found: false, running: false, synced: None });
    };
    tauri::async_runtime::spawn_blocking(move || match syncthing::synced(&syncthing::Client::new(&connection), std::path::Path::new(&path)) {
        Ok(synced) => SyncthingAt { found: true, running: true, synced: Some(synced) },
        Err(_) => SyncthingAt { found: true, running: false, synced: None },
    })
    .await
    .map_err(|e| e.to_string())
}

/// This Device's Syncthing label for the open Vault follows its Vault name (ADR 0022). Quietly
/// does nothing if Syncthing isn't there or doesn't sync it as a folder of its own.
#[tauri::command]
async fn follow_vault_name(state: State<'_, AppState>) -> Result<(), String> {
    let (root, name, manual) = with_app(&state, |app| {
        let vault = open_vault_of(app)?;
        Ok((vault.root.clone(), vault.name(), app.device.syncthing.clone()))
    })?;
    let Some(connection) = manual.or_else(syncthing::find) else { return Ok(()) };
    let _ = tauri::async_runtime::spawn_blocking(move || syncthing::follow_name(&syncthing::Client::new(&connection), &root, &name)).await;
    Ok(())
}

/// Renames a Known Vault, open or not: its new Vault name, as kept.
#[tauri::command]
fn rename_vault(state: State<AppState>, path: String, name: String) -> Result<String, String> {
    with_app(&state, |app| {
        let root = PathBuf::from(&path);
        let name = match app.vault.as_mut().filter(|v| v.root == root) {
            Some(open) => {
                open.set_name(&name, now_ms())?;
                open.name()
            }
            None => vault::rename_closed(&root, app.device.device, &name, now_ms())?,
        };
        app.device.rename_known(&root, &name);
        app.device.save()?;
        Ok(name)
    })
}

/// Takes a Vault off this Device's Known Vaults. Nothing in it is touched.
#[tauri::command]
fn forget_vault(state: State<AppState>, path: String) -> Result<(), String> {
    with_app(&state, |app| {
        let root = PathBuf::from(&path);
        if app.vault.as_ref().is_some_and(|v| v.root == root) {
            return Err("The open Vault can't be taken off the list: it's in use.".into());
        }
        app.device.forget_vault(&root);
        app.device.save()
    })
}

/// A Known Vault whose folder went missing is at `found_at` now: its new place, and its name as
/// kept there.
#[tauri::command]
fn locate_vault(state: State<AppState>, path: String, found_at: String) -> Result<String, String> {
    let to = PathBuf::from(&found_at);
    if !to.join(vault::VAULT_FILE).exists() {
        return Err(format!("{found_at} isn't a Vault: there's no {} in it.", vault::VAULT_FILE));
    }
    with_app(&state, |app| {
        let name = vault::read_name(&to, app.device.device)?;
        if app.device.known_vaults.iter().any(|k| k.path == to) {
            // On the list already: the missing entry just goes.
            app.device.forget_vault(std::path::Path::new(&path));
        } else {
            app.device.relocate_vault(std::path::Path::new(&path), &to);
        }
        app.device.rename_known(&to, &name);
        app.device.save()?;
        Ok(name)
    })
}

/// Opens a Known Vault's folder in the system's file manager.
#[cfg(desktop)]
#[tauri::command]
fn show_vault_folder(handle: AppHandle, state: State<AppState>, path: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    // Only a folder on the list: the interface can't open just any path this way.
    let known = with_app(&state, |app| Ok(app.device.known_vaults.iter().any(|k| k.path.display().to_string() == path)))?;
    if !known {
        return Err("That isn't one of this Device's Vaults.".into());
    }
    handle.opener().open_path(path, None::<&str>).map_err(|e| e.to_string())
}

#[cfg(mobile)]
#[tauri::command]
fn show_vault_folder(_path: String) -> Result<(), String> {
    Err("Android has no file manager to show it in.".into())
}

/// Is `place` inside `folder` (both as they are on disk; letters' case aside on Windows)?
fn inside(place: &std::path::Path, folder: &std::path::Path) -> bool {
    let same = |p: &std::path::Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let (place, folder) = (same(place), same(folder));
    if cfg!(windows) {
        let lower = |p: &std::path::Path| PathBuf::from(p.to_string_lossy().to_lowercase());
        lower(&place).starts_with(lower(&folder))
    } else {
        place.starts_with(&folder)
    }
}

/// Moves a Known Vault's folder into `to_parent`, under its own name: its new path. The open Vault
/// is closed first, everything written; the interface opens it again from its new place. How it's
/// synced decides how (v3 plan): not synced, just moved; a Syncthing folder of its own, taken out
/// of Syncthing here, moved and added back with the same ID; inside a larger Syncthing folder,
/// only moved within it, since moving it out would delete it on every other Device.
#[cfg(desktop)]
#[tauri::command]
async fn move_vault(state: State<'_, AppState>, path: String, to_parent: String) -> Result<String, String> {
    let from = PathBuf::from(&path);
    let parent = PathBuf::from(&to_parent);
    let to = vaults::moved_to(&from, &parent)?;
    let manual = with_app(&state, |app| {
        if app.vault.as_ref().is_some_and(|v| v.root == from) {
            if let Some(mut open) = app.vault.take() {
                open.close_all(now_ms());
            }
        }
        Ok(app.device.syncthing.clone())
    })?;
    let connection = manual.or_else(syncthing::find);
    let (f, t) = (from.clone(), to.clone());
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let Some(connection) = connection else { return vaults::move_folder(&f, &t) };
        let client = syncthing::Client::new(&connection);
        match syncthing::synced(&client, &f) {
            Err(syncthing::ApiError::NotRunning) => Err("Syncthing is set up on this Device but isn't running. Start it first, so it can follow the move.".into()),
            Err(e) => Err(syncthing::describe(e)),
            Ok(syncthing::Synced::NotSynced) => vaults::move_folder(&f, &t),
            Ok(syncthing::Synced::Own { id, .. }) => syncthing::move_own_folder(&client, &id, &t, || vaults::move_folder(&f, &t)),
            Ok(syncthing::Synced::Inside { label, path, .. }) => {
                if inside(&parent, std::path::Path::new(&path)) {
                    vaults::move_folder(&f, &t)
                } else {
                    Err(format!(
                        "This Vault is synced as part of the Syncthing folder “{label}”. Moving it out of there would delete it on your other Devices. Move it somewhere inside {path}, or make it a Syncthing folder of its own first.",
                    ))
                }
            }
        }
    })
    .await
    .map_err(|e| e.to_string())??;
    with_app(&state, |app| {
        app.device.relocate_vault(&from, &to);
        app.device.save()
    })?;
    Ok(to.display().to_string())
}

#[cfg(mobile)]
#[tauri::command]
async fn move_vault(_path: String, _to_parent: String) -> Result<String, String> {
    Err("Vaults aren't moved on Android.".into())
}

#[tauri::command]
fn set_device_name(state: State<AppState>, name: String) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 100 {
        return Err("A device name needs 1 to 100 characters.".into());
    }
    with_app(&state, |app| open_vault_of(app)?.set_device_name(name, now_ms()))
}

#[tauri::command]
async fn list_works(handle: AppHandle) -> Result<Listing, String> {
    off_main(handle, move |app| Ok(open_vault_of(app)?.list(now_ms()))).await
}

#[tauri::command]
async fn open_work(handle: AppHandle, id: String) -> Result<OpenedWork, String> {
    off_main(handle, move |app| open_vault_of(app)?.open_work(&id, now_ms())).await
}

/// A Work's own text, for Find in Vault, unless it still has the stamp `known`. It runs off the
/// main thread, and takes the app's lock only to find the Work's files: reading a History can
/// take a while, and typing in an open Work mustn't wait for it.
#[tauri::command]
async fn text_for_finding(handle: AppHandle, id: String, known: Option<String>) -> Result<vault::TextForFinding, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let source = with_app(&handle.state::<AppState>(), |app| open_vault_of(app)?.text_source(&id))?;
        Ok(source.read(known.as_deref()))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// A Work for the Reading panel (vault/reading.rs), unless it still has the stamp `known`: found
/// by its Work ID (`identity`) if it has one, else by `work`. It's never opened, and nothing is
/// recorded. The file is read without the app's lock, as for Find in Vault.
#[tauri::command]
async fn read_work(handle: AppHandle, work: String, identity: Option<String>, known: Option<String>) -> Result<vault::Reading, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = handle.state::<AppState>();
        let source = with_app(&state, |app| Ok(open_vault_of(app)?.reading_source(&work, identity.as_deref())))?;
        let loaded = source.load();
        with_app(&state, |app| Ok(open_vault_of(app)?.reading(source, loaded, known.as_deref())))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Makes a new, empty Work and opens it.
#[tauri::command]
fn create_work(state: State<AppState>, id: String) -> Result<OpenedWork, String> {
    with_app(&state, |app| open_vault_of(app)?.create_work(&id, now_ms()))
}

#[tauri::command]
fn close_work(state: State<AppState>, id: String) -> Result<(), String> {
    with_app(&state, |app| {
        open_vault_of(app)?.close_work(&id, now_ms());
        Ok(())
    })
}

/// Edits from the editor, in the order they were made.
#[tauri::command]
fn edit(state: State<AppState>, id: String, ops: Vec<Op>) -> Result<WorkState, String> {
    with_app(&state, |app| open_vault_of(app)?.edit(&id, ops, now_ms()))
}

/// The Author left the Work for now: the Text file is brought up to date.
#[tauri::command]
fn leave_work(state: State<AppState>, id: String) -> Result<WorkState, String> {
    with_app(&state, |app| open_vault_of(app)?.leave(&id, now_ms()))
}

#[derive(Serialize)]
struct WorkText {
    text: String,
    state: WorkState,
}

/// The whole text, for an editor that fell out of step with the History.
#[tauri::command]
fn work_text(state: State<AppState>, id: String) -> Result<WorkText, String> {
    with_app(&state, |app| {
        let (text, state) = open_vault_of(app)?.work_text(&id, now_ms())?;
        Ok(WorkText { text, state })
    })
}

/// The Inclusions of a Work (open or not) that can't be shown now, each as "label: why": an
/// Export asks before going ahead without them.
#[tauri::command]
fn missing_inclusions(state: State<AppState>, id: String) -> Result<Vec<String>, String> {
    with_app(&state, |app| open_vault_of(app)?.missing_inclusions(&id))
}

/// A new Comment on included text (`from..to`, UTF-16, in the text of the Inclusion whose
/// character is at `at` in the Work `id`): kept in the Work it includes (ADR 0015).
#[tauri::command]
fn comment_on_inclusion(state: State<AppState>, id: String, at: usize, from: usize, to: usize, sender: String, body: String) -> Result<(), String> {
    with_app(&state, |app| open_vault_of(app)?.comment_on_inclusion(&id, at, from, to, &sender, &body, now_ms()))
}

/// A new Annotation (ADR 0020) by the Work `id` on included text (`from..to`, UTF-16, in the
/// text of the Inclusion whose character is at `at`). Returns its ID.
#[tauri::command]
fn annotate(state: State<AppState>, id: String, at: usize, from: usize, to: usize, body: String) -> Result<String, String> {
    with_app(&state, |app| open_vault_of(app)?.annotate(&id, at, from, to, &body, now_ms()))
}

/// A new Annotation by the Work `id` on a whole Annotation that `owner` (a Work's ID) made.
#[tauri::command]
fn annotate_annotation(state: State<AppState>, id: String, owner: String, on: String, body: String) -> Result<String, String> {
    with_app(&state, |app| open_vault_of(app)?.annotate_annotation(&id, &owner, &on, &body, now_ms()))
}

/// New text for one of the Work's Annotations.
#[tauri::command]
fn set_annotation(state: State<AppState>, id: String, annotation: String, body: String) -> Result<(), String> {
    with_app(&state, |app| open_vault_of(app)?.set_annotation_body(&id, &annotation, &body, now_ms()))
}

/// Deletes one of the Work's Annotations.
#[tauri::command]
fn delete_annotation(state: State<AppState>, id: String, annotation: String) -> Result<(), String> {
    with_app(&state, |app| open_vault_of(app)?.delete_annotation(&id, &annotation, now_ms()))
}

/// The Work's Detached annotations.
#[tauri::command]
fn detached_annotations(state: State<AppState>, id: String) -> Result<Vec<vault::DetachedView>, String> {
    with_app(&state, |app| open_vault_of(app)?.detached_annotations(&id))
}

/// A Comment's Replies and settings changed in a Work that may not be open (from the Comments
/// panel of a Work that includes it).
#[tauri::command]
fn comment_in(state: State<AppState>, work: String, ops: Vec<Op>) -> Result<(), String> {
    with_app(&state, |app| open_vault_of(app)?.comment_in(&work, ops, now_ms()))
}

/// The Author saw the note about a new Chapter after an Inclusion (its character at `at` in the
/// Work `id`), and included it too or dismissed it: the note goes.
#[tauri::command]
fn settle_split(state: State<AppState>, id: String, at: usize) -> Result<(), String> {
    with_app(&state, |app| open_vault_of(app)?.settle_split(&id, at, now_ms()))
}

/// Gives the Inclusion whose character is at `at` in the Work `id` its own Styles (by name).
#[tauri::command]
fn set_inclusion_styles(state: State<AppState>, id: String, at: usize, styles: Vec<String>) -> Result<(), String> {
    with_app(&state, |app| open_vault_of(app)?.set_inclusion_styles(&id, at, &styles, now_ms()))
}

/// Can the Work `work` be included in the open Work `id`? An error saying why not (it would
/// make a Work include itself). Asked before the Inclusion is put in the text.
#[tauri::command]
fn check_inclusion(state: State<AppState>, id: String, work: String) -> Result<(), String> {
    with_app(&state, |app| open_vault_of(app)?.check_inclusion(&id, &work))
}

/// "New Collection from selected Works…": makes a Work `id` holding Inclusions of `works`, in
/// that order, and opens it.
#[tauri::command]
async fn create_collection(handle: AppHandle, id: String, works: Vec<String>) -> Result<OpenedWork, String> {
    off_main(handle, move |app| open_vault_of(app)?.create_collection(&id, &works, now_ms())).await
}

/// The Chapters of a Work (open or not), to pick one to include.
#[tauri::command]
fn chapters_of(state: State<AppState>, work: String) -> Result<Vec<vault::ChapterChoice>, String> {
    with_app(&state, |app| open_vault_of(app)?.chapters_of(&work))
}

/// An open Work's Inclusions (ADR 0015), with the included text as it is now, for the editor.
#[tauri::command]
async fn get_inclusions(handle: AppHandle, id: String) -> Result<Vec<vault::InclusionView>, String> {
    off_main(handle, move |app| open_vault_of(app)?.inclusions(&id)).await
}

/// Typesets a Work's text as it is now, the open burst included, for the Page preview. It runs
/// off the main thread, and takes the app's lock only to read the text.
/// `look`: the Look to render (picked in the preview, remembered on this Device), else the Work's first.
/// `milestone`: render the Work as it was at that Milestone instead (its pages are then under
/// the preview ID `<id>@<milestone>`, apart from the Work's own).
#[tauri::command]
async fn compile_preview(handle: AppHandle, id: String, look: Option<String>, milestone: Option<String>) -> Result<typeset::Compiled, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let (text, header, blocks, fonts_dir) = with_app(&handle.state::<AppState>(), |app| {
            let vault = open_vault_of(app)?;
            let (text, header, blocks) = match &milestone {
                Some(m) => vault.typeset_input_at(&id, m, look.as_deref(), now_ms())?,
                None => vault.typeset_input(&id, look.as_deref(), now_ms())?,
            };
            Ok((text, header, blocks, vault.root.join("fonts")))
        })?;
        let key = match &milestone {
            Some(m) => format!("{id}@{m}"),
            None => id,
        };
        with_typesetter(&handle, |typesetter| Ok(typesetter.compile_body(&key, &fonts_dir, &header, text, blocks)))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// One page of a Work's Page preview, as SVG, unless it's the same as `known`.
#[tauri::command]
async fn preview_page(handle: AppHandle, id: String, page: usize, known: Option<String>) -> Result<typeset::PagePicture, String> {
    tauri::async_runtime::spawn_blocking(move || {
        with_typesetter(&handle, |typesetter| typesetter.page(&id, page, known.as_deref()))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// What a PDF Export did, for the line that says so.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Exported {
    path: String,
    pages: usize,
    /// Fonts that came from this Device: another Device's PDF of it would differ.
    device_fonts: Vec<String>,
    /// Inclusions that couldn't be shown (each with why): the PDF has a line saying so in their place.
    missing: Vec<String>,
}

/// Exports a Work as a PDF in `look` (else its first). Changed Styles are recorded first, and
/// it's refused (before asking where to save) if the Work doesn't typeset. None if the Author
/// cancelled the save dialog.
#[tauri::command]
async fn export_pdf(handle: AppHandle, id: String, look: Option<String>, milestone: Option<String>) -> Result<Option<Exported>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let (input, fonts_dir) = with_app(&handle.state::<AppState>(), |app| {
            let vault = open_vault_of(app)?;
            let input = match &milestone {
                Some(m) => vault.export_input_at(&id, m, vault::Export { look: look.as_deref(), ..vault::Export::default() }, now_ms())?,
                None => vault.export_input(&id, vault::Export { look: look.as_deref(), ..vault::Export::default() }, now_ms())?,
            };
            Ok((input, vault.root.join("fonts")))
        })?;
        let pdf = with_typesetter(&handle, |typesetter| typesetter.export_pdf(&fonts_dir, &input.header, input.text, input.date_ms, &input.ident))?;
        let name = id.rsplit('/').next().unwrap_or(&id);
        let file_name = match &look {
            Some(look) => format!("{name} ({look}).pdf"),
            None => format!("{name}.pdf"),
        };
        let Some(path) = handle.dialog().file().set_title("Export as PDF").set_file_name(&file_name)
            .add_filter("PDF", &["pdf"]).blocking_save_file().and_then(|p| p.into_path().ok())
        else {
            return Ok(None);
        };
        std::fs::write(&path, &pdf.bytes).map_err(|e| format!("Couldn't write {}: {e}", path.display()))?;
        Ok(Some(Exported { path: path.display().to_string(), pages: pdf.pages, device_fonts: pdf.device_fonts, missing: input.missing }))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// What a `.docx` Export did.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportedDocx {
    path: String,
    /// What the Author may want to know, and Inclusions that couldn't be shown: a line each,
    /// with the library file to open to fix it, if there's one (see `docx::report`).
    report: Vec<docx::Line>,
}

/// Exports a Work as a `.docx` in `look` (by name; None, its first), or with `plain`, in no
/// Look, keeping only its prose and structure (ADR 0016). With `comments`, its open Comments
/// (and those on the text it includes) go in as Word comments; never from a Milestone. Changed
/// Styles are recorded first, as for a PDF. With `embed_fonts`, the fonts its text is set in
/// go in too, as far as their licences allow. None if the Author cancelled the save dialog.
#[tauri::command]
async fn export_docx(handle: AppHandle, id: String, look: Option<String>, plain: bool, comments: bool, embed_fonts: bool, milestone: Option<String>) -> Result<Option<ExportedDocx>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let (input, fonts_dir) = with_app(&handle.state::<AppState>(), |app| {
            let vault = open_vault_of(app)?;
            let how = vault::Export { look: look.as_deref(), no_look: plain, comments: comments && milestone.is_none() };
            let input = match &milestone {
                Some(m) => vault.export_input_at(&id, m, how, now_ms())?,
                None => vault.export_input(&id, how, now_ms())?,
            };
            Ok((input, vault.root.join("fonts")))
        })?;
        let name = id.rsplit('/').next().unwrap_or(&id).to_string();
        let (header, options) = docx::prepare(&input, &name, plain, embed_fonts);
        let made = with_typesetter(&handle, |typesetter| typesetter.export_docx(&fonts_dir, &header, input.text, &input.inclusions, &options))?;
        let file_name = match (&look, plain) {
            (_, true) => format!("{name} (prose only).docx"),
            (Some(look), false) => format!("{name} ({look}).docx"),
            (None, false) => format!("{name}.docx"),
        };
        let Some(path) = handle.dialog().file().set_title("Export as .docx").set_file_name(&file_name)
            .add_filter("Word document", &["docx"]).blocking_save_file().and_then(|p| p.into_path().ok())
        else {
            return Ok(None);
        };
        std::fs::write(&path, &made.docx.bytes).map_err(|e| format!("Couldn't write {}: {e}", path.display()))?;
        let mut report = docx::report(&made.docx.report, &made.lines, &input.made, plain);
        report.extend(input.missing.iter().map(|m| docx::Line { text: format!("An Inclusion couldn't be shown: {m}"), open: None }));
        Ok(Some(ExportedDocx { path: path.display().to_string(), report }))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// A Work's Milestones, oldest first, for the Page preview's picker.
#[tauri::command]
fn get_milestones(state: State<AppState>, id: String) -> Result<Vec<vault::MilestoneInfo>, String> {
    with_app(&state, |app| open_vault_of(app)?.milestones(&id, now_ms()))
}

/// Names the current point in a Work's History. Returns the open Work's state.
#[tauri::command]
fn make_milestone(state: State<AppState>, id: String, name: String) -> Result<Option<WorkState>, String> {
    with_app(&state, |app| open_vault_of(app)?.make_milestone(&id, &name, now_ms()))
}

/// The History tab: an open Work's newest `limit` Sessions and all its Milestones.
#[tauri::command]
async fn get_history(handle: AppHandle, id: String, limit: usize, filtered: bool) -> Result<vault::HistoryView, String> {
    off_main(handle, move |app| open_vault_of(app)?.history(&id, limit, filtered, now_ms())).await
}

/// What some of an open Work's Changes did (a Session's), as snippets.
#[tauri::command]
async fn get_snippets(handle: AppHandle, id: String, changes: Vec<String>, filtered: bool) -> Result<vault::SnippetsView, String> {
    off_main(handle, move |app| open_vault_of(app)?.snippets(&id, &changes, filtered, now_ms())).await
}

/// What changed in an open Work from one version to another: a Milestone's ID, or none for now.
#[tauri::command]
async fn compare_versions(handle: AppHandle, id: String, from: Option<String>, to: Option<String>) -> Result<vault::ComparisonView, String> {
    off_main(handle, move |app| open_vault_of(app)?.compare_versions(&id, from.as_deref(), to.as_deref(), now_ms())).await
}

/// What reverting some of an open Work's Changes would do (the Revert itself is an op in its queue).
#[tauri::command]
async fn preview_revert(handle: AppHandle, id: String, changes: Vec<String>) -> Result<vault::RevertPreview, String> {
    off_main(handle, move |app| open_vault_of(app)?.preview_revert(&id, &changes, now_ms())).await
}

/// What restoring some of an open Work's Changes would bring back.
#[tauri::command]
async fn preview_restore(handle: AppHandle, id: String, changes: Vec<String>) -> Result<vault::RestorePreview, String> {
    off_main(handle, move |app| open_vault_of(app)?.preview_restore(&id, &changes, now_ms())).await
}

/// What returning an open Work to a Milestone would do.
#[tauri::command]
async fn preview_return(handle: AppHandle, id: String, milestone: String) -> Result<vault::ReturnPreview, String> {
    off_main(handle, move |app| open_vault_of(app)?.preview_return(&id, &milestone, now_ms())).await
}

/// Gives a Milestone of an open Work a new name. Returns the Work's state.
#[tauri::command]
fn rename_milestone(state: State<AppState>, id: String, milestone: String, name: String) -> Result<WorkState, String> {
    with_app(&state, |app| open_vault_of(app)?.rename_milestone(&id, &milestone, &name, now_ms()))
}

/// Deletes a Milestone of an open Work, or brings a deleted one back. Returns the Work's state.
#[tauri::command]
fn set_milestone_deleted(state: State<AppState>, id: String, milestone: String, deleted: bool) -> Result<WorkState, String> {
    with_app(&state, |app| open_vault_of(app)?.set_milestone_deleted(&id, &milestone, deleted, now_ms()))
}

/// Dismisses a Merge notice in an open Work (recorded, so it's gone on every Device).
#[tauri::command]
fn dismiss_notice(state: State<AppState>, id: String, key: String) -> Result<WorkState, String> {
    with_app(&state, |app| open_vault_of(app)?.dismiss_notice(&id, &key, now_ms()))
}

/// A Work's Preamble, for the dialog. The Work needn't be open.
#[tauri::command]
fn get_preamble(state: State<AppState>, id: String) -> Result<vault::PreambleView, String> {
    with_app(&state, |app| open_vault_of(app)?.preamble_for_dialog(&id, now_ms()))
}

/// Changes a Work's Preamble. Returns the open Work's state (None if it isn't open).
#[tauri::command]
fn set_preamble(state: State<AppState>, id: String, preamble: preamble::Preamble) -> Result<Option<WorkState>, String> {
    with_app(&state, |app| open_vault_of(app)?.set_preamble(&id, &preamble, now_ms()))
}

/// The Style library: its Styles, Looks and Story elements, and Conflict copies to resolve.
#[tauri::command]
async fn get_library(handle: AppHandle) -> Result<library::Library, String> {
    off_main(handle, move |app| Ok(open_vault_of(app)?.library())).await
}

/// A library file's text, for its tab.
#[tauri::command]
fn read_library(state: State<AppState>, path: String) -> Result<String, String> {
    with_app(&state, |app| library::read(&open_vault_of(app)?.root, &path))
}

/// Typst colouring for a piece of text in the Raw view or a library tab (see `colouring`).
/// It needs no Vault: it only reads the text it's given.
#[tauri::command]
fn colour_typst(text: String) -> Vec<colouring::Colour> {
    colouring::colour(&text)
}

/// The Vault's themes, for the theme picker, and the files in `themes/` that aren't themes.
#[tauri::command]
fn list_themes(state: State<AppState>) -> Result<themes::Themes, String> {
    with_app(&state, |app| Ok(themes::list(&open_vault_of(app)?.root)))
}

/// A Vault theme's CSS.
#[tauri::command]
fn read_theme(state: State<AppState>, file: String) -> Result<String, String> {
    with_app(&state, |app| themes::read(&open_vault_of(app)?.root, &file))
}

/// Picks a theme's `.css` file to import. None if the Author cancelled.
#[tauri::command]
async fn pick_theme_file(app: AppHandle) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.dialog().file().set_title("Import a theme").add_filter("Theme", &["css"]).blocking_pick_file()
            .and_then(|path| path.into_path().ok())
            .map(|path| path.display().to_string())
    })
    .await
    .map_err(|e| e.to_string())
}

/// Copies a theme into the Vault's `themes/`; one with the same name is replaced only with `replace`.
#[tauri::command]
fn import_theme(state: State<AppState>, path: String, replace: bool) -> Result<themes::Imported, String> {
    with_app(&state, |app| themes::import(&open_vault_of(app)?.root, std::path::Path::new(&path), replace))
}

// ---- Syncthing (see `syncthing`): asked without holding the app's lock, so typing never waits ----

/// The Vault's folder and how to reach Syncthing: set by hand on this Device, or found.
fn syncthing_for(state: &AppState) -> Result<(PathBuf, Option<syncthing::Connection>), String> {
    let (root, manual) = with_app(state, |app| Ok((open_vault_of(app)?.root.clone(), app.device.syncthing.clone())))?;
    Ok((root, manual.or_else(syncthing::find)))
}

/// How the Vault is syncing, for the status line and the Sync dialog.
#[tauri::command]
async fn sync_status(state: State<'_, AppState>) -> Result<syncthing::SyncStatus, String> {
    let (root, connection) = syncthing_for(&state)?;
    let Some(connection) = connection else {
        return Ok(syncthing::SyncStatus::NotFound { line: "Syncthing wasn't found".into() });
    };
    tauri::async_runtime::spawn_blocking(move || {
        let client = syncthing::Client::new(&connection);
        syncthing::status(&client, &root).unwrap_or_else(|e| syncthing::SyncStatus::from_error(e, &connection))
    })
    .await
    .map_err(|e| e.to_string())
}

/// Sets Syncthing's address and API key by hand on this Device; both empty goes back to
/// finding them in Syncthing's settings file.
#[tauri::command]
fn set_syncthing(state: State<AppState>, address: String, api_key: String) -> Result<(), String> {
    let connection = if address.trim().is_empty() && api_key.trim().is_empty() {
        None
    } else {
        let address = syncthing::local_url(&address, false)
            .ok_or("Syncthing's address must be on this Device, such as 127.0.0.1:8384 or https://127.0.0.1:8384.")?;
        if api_key.trim().is_empty() {
            return Err("The API key is missing. Syncthing shows it under Actions → Settings → GUI.".into());
        }
        Some(syncthing::Connection { address, api_key: api_key.trim().to_string() })
    };
    with_app(&state, |app| {
        app.device.syncthing = connection;
        app.device.save()
    })
}

/// The address and key set by hand on this Device, for the Sync dialog's form.
#[tauri::command]
fn get_syncthing(state: State<AppState>) -> Result<Option<syncthing::Connection>, String> {
    with_app(&state, |app| Ok(app.device.syncthing.clone()))
}

/// Shares the Vault's Syncthing folder with a Sync peer (new, or waiting to be accepted).
#[tauri::command]
async fn share_with_peer(state: State<'_, AppState>, id: String, name: String) -> Result<(), String> {
    let (root, connection) = syncthing_for(&state)?;
    let connection = connection.ok_or("Syncthing wasn't found.")?;
    tauri::async_runtime::spawn_blocking(move || syncthing::share_with(&syncthing::Client::new(&connection), &root, &id, &name))
        .await
        .map_err(|e| e.to_string())?
}

/// Makes the Vault a Syncthing folder, so it can be shared.
#[tauri::command]
async fn share_vault_with_syncthing(state: State<'_, AppState>) -> Result<(), String> {
    let (root, connection) = syncthing_for(&state)?;
    let connection = connection.ok_or("Syncthing wasn't found.")?;
    let name = with_app(&state, |app| Ok(open_vault_of(app)?.name()))?;
    tauri::async_runtime::spawn_blocking(move || syncthing::share_vault(&syncthing::Client::new(&connection), &root, &name))
        .await
        .map_err(|e| e.to_string())?
}

/// Changes the Style library (`f` gets the Vault's folder): what the Vault keeps of it is read
/// again when next needed, whether or not that worked (it may have changed some files).
fn writing_library<T>(state: &AppState, f: impl FnOnce(&std::path::Path) -> Result<T, String>) -> Result<T, String> {
    with_app(state, |app| {
        let vault = open_vault_of(app)?;
        let result = f(&vault.root);
        vault.library_written();
        result
    })
}

/// Saves a library file from its tab.
#[tauri::command]
fn write_library(state: State<AppState>, path: String, text: String) -> Result<(), String> {
    writing_library(&state, |root| library::write(root, &path, &text))
}

/// Makes a new Style, Look or Story element. Returns its path.
#[tauri::command]
fn create_library(state: State<AppState>, kind: library::Kind, name: String, element: Option<library::NewElement>) -> Result<String, String> {
    writing_library(&state, |root| library::create(root, kind, &name, element))
}

/// Gives a library file a new title (the name shown); its file name, which Works use, stays.
#[tauri::command]
fn retitle_library(state: State<AppState>, path: String, title: String) -> Result<(), String> {
    writing_library(&state, |root| library::retitle(root, &path, &title))
}

/// Moves a library file to the Vault's trash folder.
#[tauri::command]
fn trash_library(state: State<AppState>, path: String) -> Result<(), String> {
    writing_library(&state, |root| library::trash(root, &root.join(vault::TRASH), &path))
}

/// Puts the library's defaults back as this app version ships them; changed ones go to the
/// Vault's trash folder first. Returns the paths (inside `styles/`) it wrote.
#[tauri::command]
fn restore_library_defaults(state: State<AppState>) -> Result<Vec<&'static str>, String> {
    writing_library(&state, |root| library::restore_defaults(root, &root.join(vault::TRASH)))
}

/// The defaults this app has a newer version of than the Vault was given (ADR 0012).
#[tauri::command]
async fn default_offers(handle: AppHandle) -> Result<Vec<library::Offer>, String> {
    off_main(handle, move |app| open_vault_of(app)?.default_offers(now_ms())).await
}

/// The Vault's copy of a default against the newer one, line by line.
#[tauri::command]
fn compare_default(state: State<AppState>, path: String) -> Result<Vec<library::Line>, String> {
    with_app(&state, |app| library::compare_default(&open_vault_of(app)?.root, &path))
}

/// Uses the newer version of a default; the Vault's copy goes to its trash folder.
#[tauri::command]
fn use_newer_default(state: State<AppState>, path: String) -> Result<(), String> {
    with_app(&state, |app| open_vault_of(app)?.use_newer_default(&path, now_ms()))
}

/// Keeps the Vault's copy of a default: this newer version isn't offered again.
#[tauri::command]
fn keep_default(state: State<AppState>, path: String) -> Result<(), String> {
    with_app(&state, |app| open_vault_of(app)?.keep_default(&path, now_ms()))
}

/// Keeps one version of a library file that has a Conflict copy.
#[tauri::command]
fn resolve_library(state: State<AppState>, copy: String, keep: library::Keep) -> Result<(), String> {
    writing_library(&state, |root| library::resolve(root, &copy, keep))
}

/// What can go at a place in a Work's text (UTF-16), after a `#`: its Story elements and
/// Typst's functions. Works with the Page preview closed too.
#[tauri::command]
async fn complete_at(handle: AppHandle, id: String, at: usize, explicit: bool, look: Option<String>) -> Result<Option<typeset::Completions>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let (text, header, blocks, fonts_dir) = with_app(&handle.state::<AppState>(), |app| {
            let vault = open_vault_of(app)?;
            let (text, header, blocks) = vault.typeset_input(&id, look.as_deref(), now_ms())?;
            Ok((text, header, blocks, vault.root.join("fonts")))
        })?;
        with_typesetter(&handle, |typesetter| Ok(typesetter.complete(&id, &fonts_dir, &header, &text, &blocks, at, explicit)))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Where a click on a page of the Page preview leads (points from the page's top left).
#[tauri::command]
async fn preview_jump(handle: AppHandle, id: String, page: usize, x: f64, y: f64) -> Result<Option<typeset::JumpTo>, String> {
    tauri::async_runtime::spawn_blocking(move || with_typesetter(&handle, |typesetter| Ok(typesetter.jump(&id, page, x, y))))
        .await
        .map_err(|e| e.to_string())?
}

/// Where a place in a Work's text (UTF-16) is on its pages, for keeping the cursor in view.
#[tauri::command]
async fn preview_locate(handle: AppHandle, id: String, at: usize) -> Result<Option<typeset::Spot>, String> {
    tauri::async_runtime::spawn_blocking(move || with_typesetter(&handle, |typesetter| Ok(typesetter.locate(&id, at))))
        .await
        .map_err(|e| e.to_string())?
}

/// A Work's tab closed, or the preview was: its pages aren't needed any more.
#[tauri::command]
fn forget_preview(handle: AppHandle, id: String) {
    let _ = with_typesetter(&handle, |typesetter| {
        typesetter.forget(&id);
        Ok(())
    });
}

/// What the timer did to a Work, for the editor.
#[derive(Serialize, Clone)]
struct WorkStatus {
    id: String,
    state: WorkState,
}

/// About once a second: close bursts after a pause, retry saves, write Text files when due.
/// It runs here rather than in the interface, whose timers slow down in a background window.
fn start_timer(handle: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(1));
        let state = handle.state::<AppState>();
        let ticked = with_app(&state, |app| Ok(app.vault.as_mut().map(|v| (v.tick(now_ms()), v.root.clone()))));
        let Ok(Some((changed, root))) = ticked else { continue };
        for (id, state) in changed {
            let _ = handle.emit("work-status", WorkStatus { id, state });
        }
        // The Style library's and the themes' files are looked at without the app's lock (that
        // walks their folders), and compared with what was seen last under it.
        let (library, themes) = (library::stamp(&root), themes::stamp(&root));
        let (library_changed, themes_changed) = with_app(&state, |app| Ok(match app.vault.as_mut() {
            Some(vault) if vault.root == root => (vault.library_changed_to(library), vault.themes_changed_to(themes)),
            _ => (false, false), // another Vault was opened meanwhile: next time
        })).unwrap_or((false, false));
        // The Style library changed (in the app, in another editor, or from another Device).
        if library_changed {
            let _ = handle.emit("library-changed", ());
        }
        // A theme in `themes/` was added, changed or removed.
        if themes_changed {
            let _ = handle.emit("themes-changed", ());
        }
    });
}

/// The app went to the background (Android): everything only in memory goes to disk now.
#[tauri::command]
fn app_hidden(state: State<AppState>) -> Result<Vec<WorkStatus>, String> {
    let paused = with_app(&state, |app| Ok(app.vault.as_mut().map(|v| v.pause_all(now_ms())).unwrap_or_default()))?;
    Ok(paused.into_iter().map(|(id, state)| WorkStatus { id, state }).collect())
}

#[tauri::command]
fn delete_leftover(state: State<AppState>, id: String) -> Result<(), String> {
    with_app(&state, |app| open_vault_of(app)?.delete_leftover(&id))
}

/// Renames a Work; an open one keeps its tab. Returns its new ID.
#[tauri::command]
async fn rename_work(handle: AppHandle, id: String, name: String) -> Result<String, String> {
    off_main(handle, move |app| open_vault_of(app)?.rename_work(&id, &name)).await
}

/// Moves a Work to the Vault's trash.
#[tauri::command]
async fn trash_work(handle: AppHandle, id: String) -> Result<(), String> {
    off_main(handle, move |app| open_vault_of(app)?.trash_work(&id, now_ms())).await
}

#[tauri::command]
async fn list_trash(handle: AppHandle) -> Result<Vec<vault::TrashedWork>, String> {
    off_main(handle, move |app| Ok(open_vault_of(app)?.list_trash())).await
}

/// Puts a Work back from the trash. Returns the ID it has now.
#[tauri::command]
async fn restore_work(handle: AppHandle, id: String) -> Result<String, String> {
    off_main(handle, move |app| open_vault_of(app)?.restore_work(&id)).await
}

/// Restores the Work of an open tab that went to the trash on another Device.
#[tauri::command]
async fn restore_open(handle: AppHandle, id: String) -> Result<String, String> {
    off_main(handle, move |app| open_vault_of(app)?.restore_open(&id)).await
}

/// Makes a new, empty folder for Works. Returns its path.
#[tauri::command]
fn create_folder(state: State<AppState>, path: String) -> Result<String, String> {
    with_app(&state, |app| open_vault_of(app)?.create_folder(&path))
}

/// Renames (or moves) a folder. Returns the old and new ID of each Work open in it.
#[tauri::command]
async fn rename_folder(handle: AppHandle, path: String, name: String) -> Result<Vec<(String, String)>, String> {
    off_main(handle, move |app| open_vault_of(app)?.rename_folder(&path, &name, now_ms())).await
}

/// Adds a Vault Mod (such as Colours) to the whole Vault, or removes it.
#[tauri::command]
fn set_vault_mod(state: State<AppState>, name: String, on: bool) -> Result<vault::Mods, String> {
    with_app(&state, |app| open_vault_of(app)?.set_vault_mod(&name, on, now_ms()))
}

/// Gives a Work a colour (one of the Reviewers' slots), or none ("").
#[tauri::command]
fn set_work_colour(state: State<AppState>, id: String, colour: String) -> Result<(), String> {
    with_app(&state, |app| open_vault_of(app)?.set_work_colour(&id, &colour, now_ms()))
}

/// Gives a folder a colour, or none ("").
#[tauri::command]
fn set_folder_colour(state: State<AppState>, path: String, colour: String) -> Result<(), String> {
    with_app(&state, |app| open_vault_of(app)?.set_folder_colour(&path, &colour, now_ms()))
}

/// Hides a folder in the Vault browser, or shows it again. Returns the folders hidden now.
#[tauri::command]
fn set_folder_hidden(state: State<AppState>, path: String, hidden: bool) -> Result<Vec<String>, String> {
    with_app(&state, |app| open_vault_of(app)?.set_folder_hidden(&path, hidden, now_ms()))
}

/// This Device keeps its own list of hidden folders, or goes back to the Vault's.
#[tauri::command]
fn set_hidden_folders_here(state: State<AppState>, here: bool) -> Result<vault::Settings, String> {
    with_app(&state, |app| open_vault_of(app)?.set_hidden_folders_here(here, now_ms()))
}

/// Moves a folder, with everything in it, to the Vault's trash.
#[tauri::command]
async fn trash_folder(handle: AppHandle, path: String) -> Result<(), String> {
    off_main(handle, move |app| open_vault_of(app)?.trash_folder(&path, now_ms())).await
}

#[tauri::command]
async fn empty_trash(handle: AppHandle) -> Result<(), String> {
    off_main(handle, move |app| open_vault_of(app)?.empty_trash()).await
}

/// Closes a tab whose Work is gone from the Vault, dropping what was typed since.
#[tauri::command]
fn discard_work(state: State<AppState>, id: String) -> Result<(), String> {
    with_app(&state, |app| {
        open_vault_of(app)?.discard_work(&id);
        Ok(())
    })
}

/// Saves the text of a tab whose Work is gone as a new Work.
#[tauri::command]
async fn save_as_new(handle: AppHandle, id: String, name: String) -> Result<OpenedWork, String> {
    off_main(handle, move |app| open_vault_of(app)?.save_as_new(&id, &name, now_ms())).await
}

/// The Mods of a Work (if `id` is given), the Mods for new Works, and those hidden on this Device.
#[tauri::command]
fn get_mods(state: State<AppState>, id: Option<String>) -> Result<vault::Mods, String> {
    with_app(&state, |app| open_vault_of(app)?.mods(id.as_deref()))
}

/// Adds a Mod to a Work, or removes it.
#[tauri::command]
fn set_mod(state: State<AppState>, id: String, name: String, on: bool) -> Result<vault::Mods, String> {
    with_app(&state, |app| open_vault_of(app)?.set_mod(&id, &name, on, now_ms()))
}

/// Whether new Works start with a Mod.
#[tauri::command]
fn set_new_work_mod(state: State<AppState>, name: String, on: bool) -> Result<vault::Mods, String> {
    with_app(&state, |app| open_vault_of(app)?.set_new_work_mod(&name, on, now_ms()))
}

/// The Mods' own settings: the Vault's, a Work's (if `id` is given) and new Works', and how
/// many Marks each Mod has dismissed in the Work.
#[tauri::command]
fn get_mod_settings(state: State<AppState>, id: Option<String>) -> Result<vault::ModSettings, String> {
    with_app(&state, |app| open_vault_of(app)?.mod_settings(id.as_deref()))
}

/// Sets a Mod's setting ("<mod>.<name>"; "" unsets it) for the Vault ("vault"), a Work ("work")
/// or new Works ("newWorks").
#[tauri::command]
fn set_mod_setting(state: State<AppState>, scope: String, id: Option<String>, key: String, value: String) -> Result<vault::ModSettings, String> {
    with_app(&state, |app| open_vault_of(app)?.set_mod_setting(&scope, id.as_deref(), &key, &value, now_ms()))
}

/// Shows a Mod's Dismissed Marks in a Work again.
#[tauri::command]
fn show_dismissed(state: State<AppState>, id: String, name: String) -> Result<vault::ModSettings, String> {
    with_app(&state, |app| open_vault_of(app)?.show_dismissed(&id, &name, now_ms()))
}

/// Hides a Mod on this Device, or shows it again.
#[tauri::command]
fn set_mod_hidden(state: State<AppState>, name: String, hidden: bool) -> Result<vault::Mods, String> {
    with_app(&state, |app| open_vault_of(app)?.set_mod_hidden(&name, hidden, now_ms()))
}

#[tauri::command]
fn get_settings(state: State<AppState>) -> Result<vault::Settings, String> {
    with_app(&state, |app| Ok(open_vault_of(app)?.settings()))
}

/// Sets a Vault setting for every Device, or (`here`) only for this one.
#[tauri::command]
fn set_setting(state: State<AppState>, name: String, value: String, here: bool) -> Result<vault::Settings, String> {
    with_app(&state, |app| open_vault_of(app)?.set_setting(&name, &value, here, now_ms()))
}

/// This Device gets its own value of a Vault setting, or goes back to the shared one.
#[tauri::command]
fn set_setting_here(state: State<AppState>, name: String, here: bool) -> Result<vault::Settings, String> {
    with_app(&state, |app| open_vault_of(app)?.set_setting_here(&name, here, now_ms()))
}

/// Gives commands keyboard shortcuts: [command, key] pairs; key "none" for no key, "" for the default.
#[tauri::command]
fn set_shortcuts(state: State<AppState>, keys: Vec<(String, String)>) -> Result<vault::Settings, String> {
    with_app(&state, |app| open_vault_of(app)?.set_shortcuts(&keys, now_ms()))
}

#[tauri::command]
fn reset_shortcuts(state: State<AppState>) -> Result<vault::Settings, String> {
    with_app(&state, |app| open_vault_of(app)?.reset_shortcuts(now_ms()))
}

/// This Device gets its own shortcuts, or goes back to the Vault's.
#[tauri::command]
fn set_shortcuts_here(state: State<AppState>, here: bool) -> Result<vault::Settings, String> {
    with_app(&state, |app| open_vault_of(app)?.set_shortcuts_here(here, now_ms()))
}

/// Every Reviewer in the Vault, removed ones too, by name.
#[tauri::command]
fn get_reviewers(state: State<AppState>) -> Result<Vec<vault::Reviewer>, String> {
    with_app(&state, |app| Ok(open_vault_of(app)?.reviewers()))
}

#[tauri::command]
fn add_reviewer(state: State<AppState>, name: String, role: String) -> Result<vault::Reviewer, String> {
    with_app(&state, |app| open_vault_of(app)?.add_reviewer(&name, &role, now_ms()))
}

#[tauri::command]
fn update_reviewer(state: State<AppState>, id: String, name: String, role: String, colour: String) -> Result<vault::Reviewer, String> {
    with_app(&state, |app| open_vault_of(app)?.update_reviewer(&id, &name, &role, &colour, now_ms()))
}

/// Removes a Reviewer (no longer offered as a sender; their Comments stay as they are), or brings one back.
#[tauri::command]
fn set_reviewer_removed(state: State<AppState>, id: String, removed: bool) -> Result<vault::Reviewer, String> {
    with_app(&state, |app| open_vault_of(app)?.set_reviewer_removed(&id, removed, now_ms()))
}

/// The tabs that were open in this Vault on this Device last time.
#[tauri::command]
fn get_layout(state: State<AppState>) -> Result<device::Layout, String> {
    with_app(&state, |app| {
        let key = open_vault_of(app)?.root.display().to_string();
        Ok(app.device.layouts.get(&key).cloned().unwrap_or_default())
    })
}

#[tauri::command]
fn save_layout(state: State<AppState>, layout: device::Layout) -> Result<(), String> {
    with_app(&state, |app| {
        let key = open_vault_of(app)?.root.display().to_string();
        app.device.layouts.insert(key, layout);
        app.device.save()
    })
}

/// Where the window is, for next time. A maximized window keeps the size it had before.
fn remember_window(window: &tauri::Window, device: &mut DeviceState) {
    let maximized = window.is_maximized().unwrap_or(false);
    let mut place = device.window.unwrap_or(device::WindowPlace { x: 100, y: 100, width: 1280, height: 800, maximized });
    place.maximized = maximized;
    if !maximized {
        if let (Ok(position), Ok(size)) = (window.outer_position(), window.outer_size()) {
            (place.x, place.y, place.width, place.height) = (position.x, position.y, size.width, size.height);
        }
    }
    device.window = Some(place);
    let _ = device.save();
}

/// Quitting: close every open burst, save, write the Text files, and remember the window.
fn save_before_quit(handle: &AppHandle) {
    let state = handle.state::<AppState>();
    let window = handle.get_webview_window("main").map(|w| w.as_ref().window());
    let _ = with_app(&state, |app| {
        if let Some(vault) = app.vault.as_mut() {
            vault.close_all(now_ms());
        }
        if let Some(window) = &window {
            remember_window(window, &mut app.device);
        }
        Ok(())
    });
}

// ---- App updates (ADR 0014) and uninstalling ----

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppInfo {
    version: String,
    /// The app removes itself here (Windows: the portable exe has no system uninstaller).
    can_uninstall: bool,
    /// The window has a frame to choose (not on Android, which fills the screen).
    can_choose_frame: bool,
    /// The system's own frame from the next start (Device state); what the window has now is
    /// what it had when it opened.
    native_frame: bool,
}

/// The installed App version, shown in Settings, whether Settings offers "Uninstall…", and the
/// window frame this Device chose.
#[tauri::command]
fn app_info(handle: AppHandle, state: State<AppState>) -> Result<AppInfo, String> {
    let native_frame = with_app(&state, |app| Ok(app.device.native_frame))?;
    Ok(AppInfo { version: handle.package_info().version.to_string(), can_uninstall: cfg!(windows), can_choose_frame: cfg!(desktop), native_frame })
}

/// The window's frame from the next start: the system's own, or the app's (Device state).
#[tauri::command]
fn set_window_frame(state: State<AppState>, native: bool) -> Result<(), String> {
    with_app(&state, |app| {
        app.device.native_frame = native;
        app.device.save()
    })
}

#[derive(Serialize)]
struct UnsavedWork {
    vault: String,
    work: String,
}

#[derive(Serialize)]
struct UninstallCheck {
    /// Why the app can't remove itself, if it can't.
    problem: Option<String>,
    /// Works whose writing never reached their History (recovery files): uninstalling loses it.
    unsaved: Vec<UnsavedWork>,
}

const UNINSTALL_ELSEWHERE: &str = "Uninstall the app the way this system does.";

/// Before the confirmation: whether the app can delete itself, and which writing would be lost.
/// The interface has already saved everything open.
#[tauri::command]
fn uninstall_check(state: State<AppState>) -> Result<UninstallCheck, String> {
    if !cfg!(windows) {
        return Err(UNINSTALL_ELSEWHERE.into());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let problem = uninstall::ready(&exe).err();
    let kept = with_app(&state, |app| Ok(recovery::kept_all(&app.recovery_dir)))?;
    let unsaved = kept.into_iter().map(|(vault, work)| UnsavedWork { vault: vault.display().to_string(), work }).collect();
    Ok(UninstallCheck { problem, unsaved })
}

/// "Uninstall": saves everything, starts the helper that deletes the app and this Device's data
/// once the app has quit, and quits. The Vault is never touched.
#[tauri::command]
fn uninstall(handle: AppHandle) -> Result<(), String> {
    #[cfg(windows)]
    {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        uninstall::ready(&exe)?;
        let path = handle.path();
        let candidates = [path.app_local_data_dir(), path.app_data_dir(), path.app_config_dir(), path.app_cache_dir()];
        let mut paths = uninstall::ours(candidates.into_iter().flatten(), &handle.config().identifier);
        paths.push(exe.with_extension("update.exe")); // a download an update left behind
        paths.push(exe);
        save_before_quit(&handle);
        uninstall::start(&paths)?;
        handle.exit(0);
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = handle;
        Err(UNINSTALL_ELSEWHERE.into())
    }
}

/// "Check for updates": asks GitHub for the newest stable App version.
#[tauri::command]
async fn check_update(handle: AppHandle) -> Result<update::Checked, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let current = handle.package_info().version.clone();
        let found = update::check(&current);
        let state = handle.state::<UpdateState>();
        let mut kept = state.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        *kept = None;
        let current = current.to_string();
        Ok(match found? {
            update::Found::Newest => update::Checked::Newest { current },
            update::Found::NoBuild { version } => update::Checked::NoBuild { current, version },
            update::Found::Available { version, build } => {
                *kept = Some((version.clone(), build));
                update::Checked::Available { current, version }
            }
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Serialize, Clone)]
struct DownloadProgress {
    done: u64,
    total: Option<u64>,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum Installed {
    /// Desktop: the new version is in place and the app is quitting to start it.
    Restarting,
    /// Android: the APK is downloaded, for the interface to hand to Android's installer.
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    Apk { path: String },
}

/// "Download and install": downloads the update the last check found (reporting
/// "update-progress" as it goes), checks its signature and installs it. On desktop the app then
/// saves everything and starts the new version; the interface has already closed every burst.
#[tauri::command]
async fn install_update(handle: AppHandle, version: String) -> Result<Installed, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let build = {
            let state = handle.state::<UpdateState>();
            let kept = state.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            match &*kept {
                Some((found, build)) if *found == version => build.clone(),
                _ => return Err("Check for updates again first.".to_string()),
            }
        };
        #[cfg(any(windows, target_os = "linux"))]
        let exe = update::ready()?;
        let mut shown = 0;
        let bytes = update::download(&build, &version, |done, total| {
            // About every 1%: a smooth bar without flooding the page.
            let step = total.map_or(512 * 1024, |t| t / 100).max(64 * 1024);
            if done - shown >= step || Some(done) == total {
                shown = done;
                let _ = handle.emit("update-progress", DownloadProgress { done, total });
            }
        })?;
        #[cfg(any(windows, target_os = "linux"))]
        {
            update::install(&bytes, &version)?;
            save_before_quit(&handle);
            std::process::Command::new(&exe)
                .spawn()
                .map_err(|e| format!("The update is installed, but the app couldn't start again ({e}). Start it yourself."))?;
            handle.exit(0);
            Ok(Installed::Restarting)
        }
        #[cfg(target_os = "android")]
        {
            let cache = handle.path().app_cache_dir().map_err(|e| e.to_string())?;
            let apk = update::save_apk(&bytes, &cache)?;
            Ok(Installed::Apk { path: apk.display().to_string() })
        }
        #[cfg(not(any(windows, target_os = "linux", target_os = "android")))]
        {
            let _ = bytes;
            Err("The app can't update itself on this platform.".to_string())
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

// ---- Images (ADR 0021) ----

/// The extensions of the pictures an Image can hold, for the file picker.
const PICTURES: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "svg", "pdf"];

/// A picked picture's name and bytes. On the desktop it's a path; Android's picker gives a
/// content address, which the fs plugin reads.
fn read_picture(handle: &AppHandle, file: tauri_plugin_dialog::FilePath) -> Result<(String, Vec<u8>), String> {
    use tauri_plugin_fs::FsExt;
    let name = match &file {
        tauri_plugin_dialog::FilePath::Path(path) => path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        tauri_plugin_dialog::FilePath::Url(url) => content_name(url.as_str()),
    };
    let bytes = handle.fs().read(file).map_err(|e| format!("Couldn't read that picture: {e}"))?;
    Ok((name, bytes))
}

/// `%XX` escapes decoded (as `encodeURIComponent` and content addresses write them).
fn percent_decoded(text: &str) -> String {
    let raw = text.as_bytes();
    let mut bytes = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        match (raw[i], raw.get(i + 1).copied().and_then(hex), raw.get(i + 2).copied().and_then(hex)) {
            (b'%', Some(a), Some(b)) => {
                bytes.push((a * 16 + b) as u8);
                i += 3;
            }
            (b, _, _) => {
                bytes.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The file name at the end of a content address ("…/document/primary%3APictures%2Fmap.png").
fn content_name(url: &str) -> String {
    let decoded = percent_decoded(url.rsplit('/').next().unwrap_or(url));
    decoded.rsplit(['/', ':']).next().unwrap_or(&decoded).to_string()
}

/// Insert Image…: picks a picture and keeps it in the Work `id`, ready for `Op::SetImage`.
/// None if the Author cancelled.
#[tauri::command]
async fn pick_image(handle: AppHandle, id: String) -> Result<Option<vault::KeptPicture>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let picked = handle.dialog().file().set_title("Insert Image").add_filter("Pictures", PICTURES).blocking_pick_file();
        let Some(file) = picked else { return Ok(None) };
        let (name, bytes) = read_picture(&handle, file)?;
        with_app(&handle.state::<AppState>(), |app| open_vault_of(app)?.keep_picture(&id, &name, &bytes)).map(Some)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Replace…: picks a new picture for an Image (the interface checks its kind matches, or renames).
/// None if the Author cancelled.
#[tauri::command]
async fn pick_replacement(handle: AppHandle, id: String) -> Result<Option<vault::KeptPicture>, String> {
    pick_image(handle, id).await
}

/// A picture file dragged onto the text from the system (desktop), kept like a picked one. Its
/// bytes come as the request's body (not JSON), with the Work and the file's name in headers
/// (URI-encoded).
#[tauri::command]
async fn keep_image_bytes(handle: AppHandle, request: tauri::ipc::Request<'_>) -> Result<vault::KeptPicture, String> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else { return Err("No picture came.".into()) };
    let header = |key: &str| -> Result<String, String> {
        let value = request.headers().get(key).and_then(|v| v.to_str().ok()).ok_or_else(|| format!("No {key} came."))?;
        Ok(percent_decoded(value))
    };
    let (id, name, bytes) = (header("work")?, header("name")?, bytes.clone());
    off_main(handle, move |app| open_vault_of(app)?.keep_picture(&id, &name, &bytes)).await
}

/// The Images of the Work `id` and of the Works it includes, for the Images tab.
#[tauri::command]
async fn get_images(handle: AppHandle, id: String) -> Result<vault::ImagesView, String> {
    off_main(handle, move |app| open_vault_of(app)?.images(&id)).await
}

/// An Image's picture, as bytes (for its thumbnail): of the Work `id`, or of `work`, which it includes.
#[tauri::command]
async fn image_picture(handle: AppHandle, id: String, name: String, work: Option<String>) -> Result<tauri::ipc::Response, String> {
    let (bytes, _) = off_main(handle, move |app| open_vault_of(app)?.image_picture(&id, &name, work.as_deref())).await?;
    Ok(tauri::ipc::Response::new(bytes))
}

/// One of the Work's Past pictures, as bytes, for its thumbnail in the Images tab.
#[tauri::command]
async fn past_picture(handle: AppHandle, id: String, hash: String) -> Result<tauri::ipc::Response, String> {
    let bytes = off_main(handle, move |app| open_vault_of(app)?.past_picture(&id, &hash)).await?;
    Ok(tauri::ipc::Response::new(bytes))
}

/// Deletes some of the Work's Past pictures, for good (asked first). Returns how many went.
#[tauri::command]
async fn delete_past_pictures(handle: AppHandle, id: String, hashes: Vec<String>) -> Result<usize, String> {
    off_main(handle, move |app| open_vault_of(app)?.delete_past_pictures(&id, &hashes)).await
}

/// Opens the Work's Past pictures folder in the system's file manager (made if it isn't there).
#[tauri::command]
fn open_past_pictures(handle: AppHandle, state: State<AppState>, id: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let folder = with_app(&state, |app| open_vault_of(app)?.past_pictures_folder(&id))?;
    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    handle.opener().open_path(folder.display().to_string(), None::<&str>).map_err(|e| e.to_string())
}

/// "What's new": the release's page on GitHub, in the browser.
#[tauri::command]
fn open_release_page(handle: AppHandle, version: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    // Only ever a version number's page on GitHub, whatever the interface sends.
    let version = semver::Version::parse(&version).map_err(|e| e.to_string())?;
    handle.opener().open_url(update::release_page(&version.to_string()), None::<&str>).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // Device state lives outside the Vault, in this Device's local app data.
            let local = app.path().app_local_data_dir()?;
            let device = DeviceState::load_or_create(&local)?;
            // The window where it was last time (if that's still on a screen, the system keeps it so).
            // Desktop only: on Android the app always fills the screen.
            #[cfg(desktop)]
            if let Some(window) = app.get_webview_window("main") {
                // The window opens without the system's frame (tauri.conf.json): the app draws its
                // own title bar. A Device that chose the system's frame gets it back here.
                if device.native_frame {
                    let _ = window.set_decorations(true);
                }
                if let Some(place) = device.window {
                    let _ = window.set_size(tauri::PhysicalSize::new(place.width, place.height));
                    let _ = window.set_position(tauri::PhysicalPosition::new(place.x, place.y));
                    if place.maximized {
                        let _ = window.maximize();
                    }
                }
            }
            app.manage(AppState(Mutex::new(App { device, vault: None, recovery_dir: local.join("recovery") })));
            app.manage(TypesetState(Mutex::new(typeset::Typesetter::default())));
            app.manage(UpdateState(Mutex::new(None)));
            app.manage(SpellState(Mutex::new(spelling::Spelling::default())));
            app.manage(HarperState(harper::HarperThread::start())); // empty: Harper loads on the first question
            start_timer(app.handle().clone());
            Ok(())
        })
        .on_window_event(|window, event| {
            // Quitting: close every open burst, save, and write the Text files.
            if let WindowEvent::CloseRequested { .. } = event {
                save_before_quit(window.app_handle());
            }
        })
        .invoke_handler(tauri::generate_handler![
            startup,
            pick_image,
            pick_replacement,
            keep_image_bytes,
            get_images,
            image_picture,
            open_past_pictures,
            past_picture,
            delete_past_pictures,
            pick_folder,
            list_folders,
            app_hidden,
            open_vault,
            known_vaults,
            vault_folder,
            create_vault,
            syncthing_at,
            follow_vault_name,
            rename_vault,
            forget_vault,
            locate_vault,
            show_vault_folder,
            move_vault,
            set_device_name,
            list_works,
            open_work,
            create_work,
            close_work,
            delete_leftover,
            edit,
            leave_work,
            work_text,
            get_inclusions,
            chapters_of,
            text_for_finding,
            read_work,
            check_inclusion,
            create_collection,
            missing_inclusions,
            comment_on_inclusion,
            annotate,
            annotate_annotation,
            set_annotation,
            delete_annotation,
            detached_annotations,
            comment_in,
            settle_split,
            set_inclusion_styles,
            rename_work,
            trash_work,
            list_trash,
            restore_work,
            restore_open,
            empty_trash,
            create_folder,
            rename_folder,
            set_folder_hidden,
            set_vault_mod,
            set_work_colour,
            set_folder_colour,
            set_hidden_folders_here,
            trash_folder,
            discard_work,
            save_as_new,
            get_settings,
            set_setting,
            set_setting_here,
            get_mods,
            set_mod,
            set_new_work_mod,
            get_mod_settings,
            set_mod_setting,
            show_dismissed,
            set_mod_hidden,
            spell_check,
            spell_suggest,
            dictionary_for,
            list_dictionaries,
            download_dictionary,
            remove_dictionary,
            grammar_check,
            word_types,
            set_shortcuts,
            reset_shortcuts,
            set_shortcuts_here,
            get_reviewers,
            add_reviewer,
            update_reviewer,
            set_reviewer_removed,
            get_layout,
            save_layout,
            compile_preview,
            preview_page,
            preview_jump,
            preview_locate,
            complete_at,
            get_preamble,
            set_preamble,
            get_milestones,
            get_history,
            get_snippets,
            compare_versions,
            preview_revert,
            preview_restore,
            preview_return,
            rename_milestone,
            set_milestone_deleted,
            dismiss_notice,
            make_milestone,
            export_pdf,
            export_docx,
            get_library,
            colour_typst,
            sync_status,
            set_syncthing,
            get_syncthing,
            share_with_peer,
            share_vault_with_syncthing,
            list_themes,
            read_theme,
            pick_theme_file,
            import_theme,
            read_library,
            write_library,
            create_library,
            resolve_library,
            restore_library_defaults,
            default_offers,
            compare_default,
            use_newer_default,
            keep_default,
            retitle_library,
            trash_library,
            forget_preview,
            app_info,
            set_window_frame,
            uninstall_check,
            uninstall,
            check_update,
            install_update,
            open_release_page,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
