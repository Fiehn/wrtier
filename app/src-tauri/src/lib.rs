//! The Rust side of the app: it holds the History engine, the open Vault and its open Works,
//! and answers the interface. The interface only displays and captures input (ADR 0005).

pub mod colouring;
pub mod device;
pub mod docx;
pub mod folders;
pub mod library;
pub mod patch;
pub mod preamble;
pub mod positions;
pub mod recovery;
pub mod syncthing;
pub mod themes;
pub mod typeset;
pub mod uninstall;
pub mod update;
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

/// The update "Check for updates" last found (its App version and this platform's build), kept
/// for "Download and install".
struct UpdateState(Mutex<Option<(String, update::Build)>>);

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

/// On Android the Vault is picked with the app's own folder browser instead (Android's picker
/// gives no file path): see `list_folders`.
#[cfg(mobile)]
#[tauri::command]
async fn pick_folder(_app: AppHandle) -> Result<Option<String>, String> {
    Err("On Android, Vaults are chosen with the app's folder browser.".to_string())
}

/// The folder browser's view of one folder (Android).
#[tauri::command]
fn list_folders(path: String) -> Result<folders::FolderListing, String> {
    folders::list(std::path::Path::new(&path))
}

#[cfg(desktop)]
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
fn create_collection(state: State<AppState>, id: String, works: Vec<String>) -> Result<OpenedWork, String> {
    with_app(&state, |app| open_vault_of(app)?.create_collection(&id, &works, now_ms()))
}

/// The Chapters of a Work (open or not), to pick one to include.
#[tauri::command]
fn chapters_of(state: State<AppState>, work: String) -> Result<Vec<vault::ChapterChoice>, String> {
    with_app(&state, |app| open_vault_of(app)?.chapters_of(&work))
}

/// An open Work's Inclusions (ADR 0015), with the included text as it is now, for the editor.
#[tauri::command]
fn get_inclusions(state: State<AppState>, id: String) -> Result<Vec<vault::InclusionView>, String> {
    with_app(&state, |app| open_vault_of(app)?.inclusions(&id))
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
        let state = handle.state::<TypesetState>();
        let mut typesetter = state.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        Ok(typesetter.compile_body(&key, &fonts_dir, &header, text, blocks))
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
                Some(m) => vault.export_input_at(&id, m, look.as_deref(), now_ms())?,
                None => vault.export_input(&id, look.as_deref(), now_ms())?,
            };
            Ok((input, vault.root.join("fonts")))
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
    paragraphs: usize,
    /// What the `.docx` couldn't carry, with its line in the text.
    left_out: Vec<docx::LeftOut>,
    /// Inclusions that couldn't be shown (each with why).
    missing: Vec<String>,
}

/// Exports a Work as a plain `.docx` in standard manuscript format (`font`: "Times New Roman"
/// or "Courier New"). Changed Styles are recorded first, so its Story elements do what their
/// recorded definitions say. None if the Author cancelled the save dialog.
#[tauri::command]
async fn export_docx(handle: AppHandle, id: String, font: String, milestone: Option<String>) -> Result<Option<ExportedDocx>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        if !["Times New Roman", "Courier New"].contains(&font.as_str()) {
            return Err(format!("Not a manuscript font: {font}"));
        }
        let input = with_app(&handle.state::<AppState>(), |app| {
            let vault = open_vault_of(app)?;
            match &milestone {
                Some(m) => vault.export_input_at(&id, m, None, now_ms()),
                None => vault.export_input(&id, None, now_ms()),
            }
        })?;
        let name = id.rsplit('/').next().unwrap_or(&id).to_string();
        let title = if input.preamble.title.trim().is_empty() { name.clone() } else { input.preamble.title.clone() };
        let manuscript = docx::Manuscript {
            title: &title, author: &input.preamble.author, language: &input.preamble.language,
            font: &font, date_ms: input.date_ms,
        };
        let made = docx::docx(&input.docx_text, &input.header.elements, &manuscript)?;
        let Some(path) = handle.dialog().file().set_title("Export as .docx").set_file_name(format!("{name}.docx"))
            .add_filter("Word document", &["docx"]).blocking_save_file().and_then(|p| p.into_path().ok())
        else {
            return Ok(None);
        };
        std::fs::write(&path, &made.bytes).map_err(|e| format!("Couldn't write {}: {e}", path.display()))?;
        Ok(Some(ExportedDocx { path: path.display().to_string(), paragraphs: made.paragraphs, left_out: made.left_out, missing: input.missing }))
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
    tauri::async_runtime::spawn_blocking(move || syncthing::share_vault(&syncthing::Client::new(&connection), &root))
        .await
        .map_err(|e| e.to_string())?
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
        let (text, header, blocks, fonts_dir) = with_app(&handle.state::<AppState>(), |app| {
            let vault = open_vault_of(app)?;
            let (text, header, blocks) = vault.typeset_input(&id, look.as_deref(), now_ms())?;
            Ok((text, header, blocks, vault.root.join("fonts")))
        })?;
        let state = handle.state::<TypesetState>();
        let mut typesetter = state.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        Ok(typesetter.complete(&id, &fonts_dir, &header, &text, &blocks, at, explicit))
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
        // A theme in `themes/` was added, changed or removed.
        if with_app(&state, |app| Ok(app.vault.as_mut().is_some_and(|v| v.themes_changed()))).unwrap_or(false) {
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
}

/// The installed App version, shown in Settings, and whether Settings offers "Uninstall…".
#[tauri::command]
fn app_info(handle: AppHandle) -> AppInfo {
    AppInfo { version: handle.package_info().version.to_string(), can_uninstall: cfg!(windows) }
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
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // Device state lives outside the Vault, in this Device's local app data.
            let local = app.path().app_local_data_dir()?;
            let device = DeviceState::load_or_create(&local)?;
            // The window where it was last time (if that's still on a screen, the system keeps it so).
            // Desktop only: on Android the app always fills the screen.
            #[cfg(desktop)]
            if let (Some(place), Some(window)) = (device.window, app.get_webview_window("main")) {
                let _ = window.set_size(tauri::PhysicalSize::new(place.width, place.height));
                let _ = window.set_position(tauri::PhysicalPosition::new(place.x, place.y));
                if place.maximized {
                    let _ = window.maximize();
                }
            }
            app.manage(AppState(Mutex::new(App { device, vault: None, recovery_dir: local.join("recovery") })));
            app.manage(TypesetState(Mutex::new(typeset::Typesetter::default())));
            app.manage(UpdateState(Mutex::new(None)));
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
            pick_folder,
            list_folders,
            app_hidden,
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
            get_inclusions,
            chapters_of,
            check_inclusion,
            create_collection,
            missing_inclusions,
            comment_on_inclusion,
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
            retitle_library,
            trash_library,
            forget_preview,
            app_info,
            uninstall_check,
            uninstall,
            check_update,
            install_update,
            open_release_page,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
