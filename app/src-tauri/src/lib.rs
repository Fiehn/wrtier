//! The Rust side of the app: it holds the History engine, the open Vault and its open Works,
//! and answers the interface. The interface only displays and captures input (ADR 0005).

pub mod device;
pub mod patch;
pub mod positions;
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
}

struct AppState(Mutex<App>);

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
        match Vault::open(&root, app.device.device, create)? {
            Opened::NotAVault => Ok(VaultOpened::NotAVault),
            Opened::Vault(vault) => {
                let device_name = vault.device_name();
                if let Some(previous) = app.vault.as_mut() {
                    previous.close_all(now_ms());
                }
                app.vault = Some(vault);
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
    });
}

#[tauri::command]
fn delete_leftover(state: State<AppState>, id: String) -> Result<(), String> {
    with_app(&state, |app| open_vault_of(app)?.delete_leftover(&id))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // Device state lives outside the Vault, in this Device's local app data.
            let device = DeviceState::load_or_create(&app.path().app_local_data_dir()?)?;
            app.manage(AppState(Mutex::new(App { device, vault: None })));
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
