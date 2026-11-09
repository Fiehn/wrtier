//! The Rust side of the app: it holds the History engine, the open Vault and its open Works,
//! and answers the interface. The interface only displays and captures input (ADR 0005).

pub mod colouring;
pub mod device;
pub mod docx;
pub mod library;
pub mod patch;
pub mod preamble;
pub mod positions;
pub mod recovery;
pub mod typeset;
pub mod vault;
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

/// The time, always from here and never from the interface.
fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Runs a command on the app's state. A panic that gets this far becomes an error message
/// instead of closing the app; the Vault code already catches panics around each Work, so its
/// state in memory is never left half-updated.
fn with_app<T>(state: &AppState, f: impl FnOnce(&mut App) -> Result<T, String>) -> Result<T, String> {
    let mut app = state.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    match catch_unwind(AssertUnwindSafe(|| f(&mut app))) {
        Ok(result) => result,
        Err(panic) => Err(format!("Something went wrong: {}", vault::panic_message(panic.as_ref()))),
    }
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
    with_app(&state, |app| Ok(Startup { last_vault: app.device.last_vault.as_ref().map(|p| p.display().to_string()) }))
}

#[tauri::command]
async fn pick_folder(app: AppHandle) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.dialog().file().set_title("Choose your Vault").blocking_pick_folder()
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
    Opened { root: String, device_name: Option<String>, suggested_name: String },
}

#[tauri::command]
fn open_vault(state: State<AppState>, path: String, create: bool) -> Result<VaultOpened, String> {
    with_app(&state, |app| {
        let root = PathBuf::from(&path);
        match Vault::open(&root, app.device.device, create, &app.recovery_dir)? {
            Opened::NotAVault => Ok(VaultOpened::NotAVault),
            Opened::Vault(vault) => {
                let device_name = vault.device_name();
                if let Some(previous) = app.vault.as_mut() {
                    previous.close_all(now_ms());
                }
                app.vault = Some(vault);
                typeset::warm_up();
                app.device.last_vault = Some(root);
                app.device.save()?;
                Ok(VaultOpened::Opened { root: path, device_name, suggested_name: device::suggested_name() })
            }
        }
    })
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
fn list_works(state: State<AppState>) -> Result<Listing, String> {
    with_app(&state, |app| Ok(open_vault_of(app)?.list(now_ms())))
}

#[tauri::command]
fn open_work(state: State<AppState>, id: String) -> Result<OpenedWork, String> {
    with_app(&state, |app| open_vault_of(app)?.open_work(&id, now_ms()))
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

/// Typesets a Work's text as it is now, the open burst included, for the Page preview. It runs
/// off the main thread, and takes the app's lock only to read the text.
/// `look`: the Look to render (picked in the preview, remembered on this Device), else the Work's first.
/// `milestone`: render the Work as it was at that Milestone instead (its pages are then under
/// the preview ID `<id>@<milestone>`, apart from the Work's own).
#[tauri::command]
async fn compile_preview(handle: AppHandle, id: String, look: Option<String>, milestone: Option<String>) -> Result<typeset::Compiled, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let (text, header, fonts_dir) = with_app(&handle.state::<AppState>(), |app| {
            let vault = open_vault_of(app)?;
            let (text, header) = match &milestone {
                Some(m) => vault.typeset_input_at(&id, m, look.as_deref(), now_ms())?,
                None => vault.typeset_input(&id, look.as_deref(), now_ms())?,
            };
            Ok((text, header, vault.root.join("fonts")))
        })?;
        let key = match &milestone {
            Some(m) => format!("{id}@{m}"),
            None => id,
        };
        let state = handle.state::<TypesetState>();
        let mut typesetter = state.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        Ok(typesetter.compile_with(&key, &fonts_dir, &header, text))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// One page of a Work's Page preview, as SVG, unless it's the same as `known`.
#[tauri::command]
async fn preview_page(handle: AppHandle, id: String, page: usize, known: Option<String>) -> Result<typeset::PagePicture, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = handle.state::<TypesetState>();
        let typesetter = state.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        typesetter.page(&id, page, known.as_deref())
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
}

/// Exports a Work as a PDF in `look` (else its first). Changed Styles are recorded first, and
/// it's refused (before asking where to save) if the Work doesn't typeset. None if the Author
/// cancelled the save dialog.
#[tauri::command]
async fn export_pdf(handle: AppHandle, id: String, look: Option<String>) -> Result<Option<Exported>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let (input, fonts_dir) = with_app(&handle.state::<AppState>(), |app| {
            let vault = open_vault_of(app)?;
            Ok((vault.export_input(&id, look.as_deref(), now_ms())?, vault.root.join("fonts")))
        })?;
        let pdf = {
            let state = handle.state::<TypesetState>();
            let mut typesetter = state.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            typesetter.export_pdf(&fonts_dir, &input.header, input.text, input.date_ms, &input.ident)?
        };
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
        Ok(Some(Exported { path: path.display().to_string(), pages: pdf.pages, device_fonts: pdf.device_fonts }))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// What a `.docx` Export did.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportedDocx {
    path: String,
    paragraphs: usize,
    /// What the `.docx` couldn't carry, with its line in the text.
    left_out: Vec<docx::LeftOut>,
}

/// Exports a Work as a plain `.docx` in standard manuscript format (`font`: "Times New Roman"
/// or "Courier New"). Changed Styles are recorded first, so its Story elements do what their
/// recorded definitions say. None if the Author cancelled the save dialog.
#[tauri::command]
async fn export_docx(handle: AppHandle, id: String, font: String) -> Result<Option<ExportedDocx>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        if !["Times New Roman", "Courier New"].contains(&font.as_str()) {
            return Err(format!("Not a manuscript font: {font}"));
        }
        let input = with_app(&handle.state::<AppState>(), |app| open_vault_of(app)?.export_input(&id, None, now_ms()))?;
        let name = id.rsplit('/').next().unwrap_or(&id).to_string();
        let title = if input.preamble.title.trim().is_empty() { name.clone() } else { input.preamble.title.clone() };
        let manuscript = docx::Manuscript {
            title: &title, author: &input.preamble.author, language: &input.preamble.language,
            font: &font, date_ms: input.date_ms,
        };
        let made = docx::docx(&input.text, &input.header.elements, &manuscript)?;
        let Some(path) = handle.dialog().file().set_title("Export as .docx").set_file_name(format!("{name}.docx"))
            .add_filter("Word document", &["docx"]).blocking_save_file().and_then(|p| p.into_path().ok())
        else {
            return Ok(None);
        };
        std::fs::write(&path, &made.bytes).map_err(|e| format!("Couldn't write {}: {e}", path.display()))?;
        Ok(Some(ExportedDocx { path: path.display().to_string(), paragraphs: made.paragraphs, left_out: made.left_out }))
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
fn get_history(state: State<AppState>, id: String, limit: usize, filtered: bool) -> Result<vault::HistoryView, String> {
    with_app(&state, |app| open_vault_of(app)?.history(&id, limit, filtered, now_ms()))
}

/// What some of an open Work's Changes did (a Session's), as snippets.
#[tauri::command]
fn get_snippets(state: State<AppState>, id: String, changes: Vec<String>, filtered: bool) -> Result<vault::SnippetsView, String> {
    with_app(&state, |app| open_vault_of(app)?.snippets(&id, &changes, filtered, now_ms()))
}

/// What changed in an open Work from one version to another: a Milestone's ID, or none for now.
#[tauri::command]
fn compare_versions(state: State<AppState>, id: String, from: Option<String>, to: Option<String>) -> Result<vault::ComparisonView, String> {
    with_app(&state, |app| open_vault_of(app)?.compare_versions(&id, from.as_deref(), to.as_deref(), now_ms()))
}

/// What reverting some of an open Work's Changes would do (the Revert itself is an op in its queue).
#[tauri::command]
fn preview_revert(state: State<AppState>, id: String, changes: Vec<String>) -> Result<vault::RevertPreview, String> {
    with_app(&state, |app| open_vault_of(app)?.preview_revert(&id, &changes, now_ms()))
}

/// What restoring some of an open Work's Changes would bring back.
#[tauri::command]
fn preview_restore(state: State<AppState>, id: String, changes: Vec<String>) -> Result<vault::RestorePreview, String> {
    with_app(&state, |app| open_vault_of(app)?.preview_restore(&id, &changes, now_ms()))
}

/// What returning an open Work to a Milestone would do.
#[tauri::command]
fn preview_return(state: State<AppState>, id: String, milestone: String) -> Result<vault::ReturnPreview, String> {
    with_app(&state, |app| open_vault_of(app)?.preview_return(&id, &milestone, now_ms()))
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
fn get_library(state: State<AppState>) -> Result<library::Library, String> {
    with_app(&state, |app| Ok(open_vault_of(app)?.library()))
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

/// Saves a library file from its tab.
#[tauri::command]
fn write_library(state: State<AppState>, path: String, text: String) -> Result<(), String> {
    with_app(&state, |app| library::write(&open_vault_of(app)?.root, &path, &text))
}

/// Makes a new Style, Look or Story element. Returns its path.
#[tauri::command]
fn create_library(state: State<AppState>, kind: library::Kind, name: String, element: Option<library::NewElement>) -> Result<String, String> {
    with_app(&state, |app| library::create(&open_vault_of(app)?.root, kind, &name, element))
}

/// Gives a library file a new title (the name shown); its file name, which Works use, stays.
#[tauri::command]
fn retitle_library(state: State<AppState>, path: String, title: String) -> Result<(), String> {
    with_app(&state, |app| library::retitle(&open_vault_of(app)?.root, &path, &title))
}

/// Moves a library file to the Vault's trash folder.
#[tauri::command]
fn trash_library(state: State<AppState>, path: String) -> Result<(), String> {
    with_app(&state, |app| {
        let root = &open_vault_of(app)?.root;
        library::trash(root, &root.join(vault::TRASH), &path)
    })
}

/// Keeps one version of a library file that has a Conflict copy.
#[tauri::command]
fn resolve_library(state: State<AppState>, copy: String, keep: library::Keep) -> Result<(), String> {
    with_app(&state, |app| library::resolve(&open_vault_of(app)?.root, &copy, keep))
}

/// What can go at a place in a Work's text (UTF-16), after a `#`: its Story elements and
/// Typst's functions. Works with the Page preview closed too.
#[tauri::command]
async fn complete_at(handle: AppHandle, id: String, at: usize, explicit: bool, look: Option<String>) -> Result<Option<typeset::Completions>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let (text, header, fonts_dir) = with_app(&handle.state::<AppState>(), |app| {
            let vault = open_vault_of(app)?;
            let (text, header) = vault.typeset_input(&id, look.as_deref(), now_ms())?;
            Ok((text, header, vault.root.join("fonts")))
        })?;
        let state = handle.state::<TypesetState>();
        let mut typesetter = state.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        Ok(typesetter.complete(&id, &fonts_dir, &header, &text, at, explicit))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Where a click on a page of the Page preview leads (points from the page's top left).
#[tauri::command]
async fn preview_jump(handle: AppHandle, id: String, page: usize, x: f64, y: f64) -> Result<Option<typeset::JumpTo>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = handle.state::<TypesetState>();
        let typesetter = state.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        typesetter.jump(&id, page, x, y)
    })
    .await
    .map_err(|e| e.to_string())
}

/// Where a place in a Work's text (UTF-16) is on its pages, for keeping the cursor in view.
#[tauri::command]
async fn preview_locate(handle: AppHandle, id: String, at: usize) -> Result<Option<typeset::Spot>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = handle.state::<TypesetState>();
        let typesetter = state.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        typesetter.locate(&id, at)
    })
    .await
    .map_err(|e| e.to_string())
}

/// A Work's tab closed, or the preview was: its pages aren't needed any more.
#[tauri::command]
fn forget_preview(state: State<TypesetState>, id: String) {
    state.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).forget(&id);
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
        let changed = with_app(&state, |app| Ok(app.vault.as_mut().map(|v| v.tick(now_ms())).unwrap_or_default()));
        for (id, state) in changed.unwrap_or_default() {
            let _ = handle.emit("work-status", WorkStatus { id, state });
        }
        // The Style library changed (in the app, in another editor, or from another Device).
        if with_app(&state, |app| Ok(app.vault.as_mut().is_some_and(|v| v.library_changed()))).unwrap_or(false) {
            let _ = handle.emit("library-changed", ());
        }
    });
}

#[tauri::command]
fn delete_leftover(state: State<AppState>, id: String) -> Result<(), String> {
    with_app(&state, |app| open_vault_of(app)?.delete_leftover(&id))
}

/// Renames a Work; an open one keeps its tab. Returns its new ID.
#[tauri::command]
fn rename_work(state: State<AppState>, id: String, name: String) -> Result<String, String> {
    with_app(&state, |app| open_vault_of(app)?.rename_work(&id, &name))
}

/// Moves a Work to the Vault's trash.
#[tauri::command]
fn trash_work(state: State<AppState>, id: String) -> Result<(), String> {
    with_app(&state, |app| open_vault_of(app)?.trash_work(&id, now_ms()))
}

#[tauri::command]
fn list_trash(state: State<AppState>) -> Result<Vec<vault::TrashedWork>, String> {
    with_app(&state, |app| Ok(open_vault_of(app)?.list_trash()))
}

/// Puts a Work back from the trash. Returns the ID it has now.
#[tauri::command]
fn restore_work(state: State<AppState>, id: String) -> Result<String, String> {
    with_app(&state, |app| open_vault_of(app)?.restore_work(&id))
}

/// Restores the Work of an open tab that went to the trash on another Device.
#[tauri::command]
fn restore_open(state: State<AppState>, id: String) -> Result<String, String> {
    with_app(&state, |app| open_vault_of(app)?.restore_open(&id))
}

/// Makes a new, empty folder for Works. Returns its path.
#[tauri::command]
fn create_folder(state: State<AppState>, path: String) -> Result<String, String> {
    with_app(&state, |app| open_vault_of(app)?.create_folder(&path))
}

/// Renames (or moves) a folder. Returns the old and new ID of each Work open in it.
#[tauri::command]
fn rename_folder(state: State<AppState>, path: String, name: String) -> Result<Vec<(String, String)>, String> {
    with_app(&state, |app| open_vault_of(app)?.rename_folder(&path, &name))
}

/// Moves a folder, with everything in it, to the Vault's trash.
#[tauri::command]
fn trash_folder(state: State<AppState>, path: String) -> Result<(), String> {
    with_app(&state, |app| open_vault_of(app)?.trash_folder(&path, now_ms()))
}

#[tauri::command]
fn empty_trash(state: State<AppState>) -> Result<(), String> {
    with_app(&state, |app| open_vault_of(app)?.empty_trash())
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
fn save_as_new(state: State<AppState>, id: String, name: String) -> Result<OpenedWork, String> {
    with_app(&state, |app| open_vault_of(app)?.save_as_new(&id, &name, now_ms()))
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // Device state lives outside the Vault, in this Device's local app data.
            let local = app.path().app_local_data_dir()?;
            let device = DeviceState::load_or_create(&local)?;
            // The window where it was last time (if that's still on a screen, the system keeps it so).
            if let (Some(place), Some(window)) = (device.window, app.get_webview_window("main")) {
                let _ = window.set_size(tauri::PhysicalSize::new(place.width, place.height));
                let _ = window.set_position(tauri::PhysicalPosition::new(place.x, place.y));
                if place.maximized {
                    let _ = window.maximize();
                }
            }
            app.manage(AppState(Mutex::new(App { device, vault: None, recovery_dir: local.join("recovery") })));
            app.manage(TypesetState(Mutex::new(typeset::Typesetter::default())));
            start_timer(app.handle().clone());
            Ok(())
        })
        .on_window_event(|window, event| {
            // Quitting: close every open burst, save, and write the Text files.
            if let WindowEvent::CloseRequested { .. } = event {
                let state = window.state::<AppState>();
                let _ = with_app(&state, |app| {
                    if let Some(vault) = app.vault.as_mut() {
                        vault.close_all(now_ms());
                    }
                    remember_window(window, &mut app.device);
                    Ok(())
                });
            }
        })
        .invoke_handler(tauri::generate_handler![
            startup,
            pick_folder,
            open_vault,
            set_device_name,
            list_works,
            open_work,
            create_work,
            close_work,
            delete_leftover,
            edit,
            leave_work,
            work_text,
            rename_work,
            trash_work,
            list_trash,
            restore_work,
            restore_open,
            empty_trash,
            create_folder,
            rename_folder,
            trash_folder,
            discard_work,
            save_as_new,
            get_settings,
            set_setting,
            get_mods,
            set_mod,
            set_new_work_mod,
            set_mod_hidden,
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
            read_library,
            write_library,
            create_library,
            resolve_library,
            retitle_library,
            trash_library,
            forget_preview,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
