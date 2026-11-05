mod novel;

use std::sync::Mutex;
use tauri::State;

// The loaded novel lives here between button presses, so recompiles can reuse earlier work.
type NovelState = Mutex<Option<novel::Novel>>;

#[tauri::command]
async fn typst_setup(fonts_dir: String, state: State<'_, NovelState>) -> Result<String, String> {
    let mut guard = state.lock().unwrap();
    let dir = fonts_dir.trim();
    let novel = novel::Novel::new(if dir.is_empty() { None } else { Some(std::path::Path::new(dir)) })?;
    let report = novel.compile()?;
    *guard = Some(novel);
    Ok(report)
}

#[tauri::command]
async fn typst_edit(state: State<'_, NovelState>) -> Result<String, String> {
    let mut guard = state.lock().unwrap();
    let novel = guard.as_mut().ok_or("press Set up first")?;
    novel.edit();
    novel.compile()
}

#[tauri::command]
async fn typst_pdf(state: State<'_, NovelState>) -> Result<String, String> {
    let guard = state.lock().unwrap();
    let novel = guard.as_ref().ok_or("press Set up first")?;
    Ok(format!("pdf export: {} ms", novel.pdf_ms()?))
}


use rusqlite::Connection;
use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};

// What we send back to the interface after every write or check.
#[derive(Serialize)]
struct Report {
    rows: i64,
    integrity: String,
    size: u64,
    modified_ms: u64,
    last_rows: Vec<String>,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

// Open the database, creating the file and its table if they don't exist yet.
// SQLite's default journal mode is what we want: no extra -wal/-shm files for Syncthing.
fn open(path: &str) -> Result<Connection, String> {
    let conn = Connection::open(path).map_err(|e| e.to_string())?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS rows (
            id INTEGER PRIMARY KEY,
            device TEXT NOT NULL,
            text TEXT NOT NULL,
            written_ms INTEGER NOT NULL
        )",
        [],
    )
    .map_err(|e| e.to_string())?;
    Ok(conn)
}
// Look at the file: is it healthy, how many rows, and its size and modified time.
fn report(path: &str) -> Result<Report, String> {
    let (rows, integrity, last_rows) = {
        let conn = open(path)?;
        let integrity: String = conn
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM rows", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT device, text FROM rows ORDER BY written_ms DESC LIMIT 5")
            .map_err(|e| e.to_string())?;
        let last_rows = stmt
            .query_map([], |r| {
                let device: String = r.get(0)?;
                let text: String = r.get(1)?;
                Ok(format!("{device}: {text}"))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<String>, _>>()
            .map_err(|e| e.to_string())?;
        (rows, integrity, last_rows)
    }; // the connection closes here, at the end of this block
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    let modified_ms = meta
        .modified()
        .map_err(|e| e.to_string())?
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_millis() as u64;

    Ok(Report { rows, integrity, size: meta.len(), modified_ms, last_rows })
}
#[tauri::command]
fn write_row(path: String, text: String) -> Result<Report, String> {
    {
        let conn = open(&path)?;
        conn.execute(
            "INSERT INTO rows (device, text, written_ms) VALUES (?1, ?2, ?3)",
            (std::env::consts::OS, &text, now_ms()),
        )
        .map_err(|e| e.to_string())?;
    } // closed straight away: we never keep the file open between writes (ADR 0002)
    report(&path)
}
#[tauri::command]
fn check(path: String) -> Result<Report, String> {
    report(&path)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
     tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(NovelState::new(None))
        .invoke_handler(tauri::generate_handler![write_row, check, typst_setup, typst_edit, typst_pdf])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}