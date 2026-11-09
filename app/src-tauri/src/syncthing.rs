//! Syncthing, as the app sees it: how the Vault is syncing, and sharing it with another Device.
//!
//! The Author installs and runs Syncthing themselves; the app never starts, stops or bundles
//! it. When it can reach Syncthing's REST API on this Device, it shows a status line and the
//! Sync dialog. It only ever changes the Vault's own Syncthing folder and the Sync peers that
//! folder is shared with, never Syncthing's other settings. Without Syncthing nothing else
//! changes.
//!
//! The address and API key come from Syncthing's own settings file in its usual places on
//! desktop. On Android that file is private to the Syncthing app, so the Author pastes the key
//! (kept as Device state); any Device can set both by hand. Only addresses on this Device are
//! used, since the API key gives full control of Syncthing.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Where to reach Syncthing's REST API.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    /// The base URL, such as `http://127.0.0.1:8384`.
    pub address: String,
    pub api_key: String,
}

// ---- Finding Syncthing ----

/// Where Syncthing keeps its settings file on this kind of Device, most likely first.
pub fn config_paths() -> Vec<PathBuf> {
    let env = |name: &str| std::env::var_os(name).map(PathBuf::from).filter(|p| !p.as_os_str().is_empty());
    let mut dirs = Vec::new();
    if let Some(home) = env("STHOMEDIR") {
        dirs.push(home);
    }
    if cfg!(windows) {
        if let Some(local) = env("LOCALAPPDATA") {
            dirs.push(local.join("Syncthing"));
        }
    } else {
        let home = env("HOME");
        // Syncthing 1.27 and later keep it in the state folder; older ones in the config folder.
        if let Some(state) = env("XDG_STATE_HOME") {
            dirs.push(state.join("syncthing"));
        }
        if let Some(home) = &home {
            dirs.push(home.join(".local/state/syncthing"));
        }
        if let Some(config) = env("XDG_CONFIG_HOME") {
            dirs.push(config.join("syncthing"));
        }
        if let Some(home) = &home {
            dirs.push(home.join(".config/syncthing"));
            dirs.push(home.join("Library/Application Support/Syncthing"));
        }
    }
    dirs.into_iter().map(|dir| dir.join("config.xml")).collect()
}

/// Syncthing's settings file, read for its web interface's address and API key.
pub fn find() -> Option<Connection> {
    config_paths().iter().find_map(|path| std::fs::read_to_string(path).ok().and_then(|xml| parse_config(&xml)))
}

/// The address and API key from a `config.xml`.
pub fn parse_config(xml: &str) -> Option<Connection> {
    let doc = roxmltree::Document::parse(xml).ok()?;
    let gui = doc.descendants().find(|n| n.has_tag_name("gui") && n.parent().is_some_and(|p| p.has_tag_name("configuration")))?;
    let child = |name: &str| gui.children().find(|n| n.has_tag_name(name)).and_then(|n| n.text()).map(str::trim).unwrap_or("");
    let tls = gui.attribute("tls").is_some_and(|v| v.eq_ignore_ascii_case("true"));
    let key = child("apikey");
    if key.is_empty() {
        return None;
    }
    let address = local_url(child("address"), tls)?;
    Some(Connection { address, api_key: key.to_string() })
}

/// A web interface address as Syncthing writes it ("127.0.0.1:8384", "0.0.0.0:8384", a URL) as
/// a URL on this Device. None for an address that isn't on this Device, or a socket.
pub fn local_url(address: &str, tls: bool) -> Option<String> {
    let address = address.trim();
    let (scheme, rest) = match address.split_once("://") {
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https") => (scheme.to_ascii_lowercase(), rest),
        Some(_) => return None, // unix:// and the like
        None => ((if tls { "https" } else { "http" }).to_string(), address),
    };
    let rest = rest.trim_end_matches('/');
    if rest.is_empty() || rest.starts_with('/') || rest.contains('/') {
        return None;
    }
    let (host, port) = match rest.rsplit_once(':') {
        Some((host, port)) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) && !host.ends_with(':') => (host, port),
        _ if rest.starts_with('[') && rest.ends_with(']') => (rest, "8384"),
        _ if !rest.contains(':') => (rest, "8384"),
        _ => return None,
    };
    let host = match host.trim_start_matches('[').trim_end_matches(']') {
        // Listening on every address includes this Device's own.
        "" | "0.0.0.0" | "::" => "127.0.0.1".to_string(),
        "localhost" => "127.0.0.1".to_string(),
        "::1" => "[::1]".to_string(),
        h if h.parse::<std::net::Ipv4Addr>().is_ok_and(|ip| ip.is_loopback()) => h.to_string(),
        _ => return None,
    };
    Some(format!("{scheme}://{host}:{port}"))
}

// ---- Talking to it ----

#[derive(Debug, PartialEq, Eq)]
pub enum ApiError {
    /// Nothing answered at the address.
    NotRunning,
    /// Syncthing answered but didn't accept the API key.
    Refused,
    Other(String),
}

impl From<ureq::Error> for ApiError {
    fn from(e: ureq::Error) -> Self {
        match e {
            ureq::Error::ConnectionFailed | ureq::Error::HostNotFound | ureq::Error::Timeout(_) => ApiError::NotRunning,
            ureq::Error::Io(e) if matches!(e.kind(), std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::TimedOut | std::io::ErrorKind::ConnectionReset) => ApiError::NotRunning,
            other => ApiError::Other(other.to_string()),
        }
    }
}

pub struct Client {
    base: String,
    key: String,
    agent: ureq::Agent,
}

impl Client {
    pub fn new(connection: &Connection) -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(2)))
            .timeout_global(Some(Duration::from_secs(6)))
            .http_status_as_error(false)
            // Only addresses on this Device are used, and Syncthing's web interface signs its
            // own certificate when it uses HTTPS (as on Android).
            .tls_config(ureq::tls::TlsConfig::builder().disable_verification(true).build())
            .build();
        Client { base: connection.address.trim_end_matches('/').to_string(), key: connection.api_key.clone(), agent: config.into() }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    fn answer(mut response: ureq::http::Response<ureq::Body>) -> Result<Value, ApiError> {
        match response.status().as_u16() {
            401 | 403 => Err(ApiError::Refused),
            200..=299 => {
                let text = response.body_mut().read_to_string().map_err(ApiError::from)?;
                if text.trim().is_empty() { Ok(Value::Null) } else { serde_json::from_str(&text).map_err(|e| ApiError::Other(e.to_string())) }
            }
            status => {
                let text = response.body_mut().read_to_string().unwrap_or_default();
                Err(ApiError::Other(format!("Syncthing answered {status}: {}", text.trim())))
            }
        }
    }

    pub fn get(&self, path: &str) -> Result<Value, ApiError> {
        Self::answer(self.agent.get(self.url(path)).header("X-API-Key", &self.key).call()?)
    }

    pub fn post(&self, path: &str, body: &Value) -> Result<Value, ApiError> {
        Self::answer(self.agent.post(self.url(path)).header("X-API-Key", &self.key).send_json(body)?)
    }

    pub fn patch(&self, path: &str, body: &Value) -> Result<Value, ApiError> {
        Self::answer(self.agent.patch(self.url(path)).header("X-API-Key", &self.key).send_json(body)?)
    }
}

/// A query parameter, escaped.
fn param(value: &str) -> String {
    value.bytes().map(|b| match b {
        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
        _ => format!("%{b:02X}"),
    }).collect()
}

// ---- What the app shows ----

/// A Sync peer the Vault's folder is shared with.
#[derive(Serialize, Debug, PartialEq, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Peer {
    pub id: String,
    pub name: String,
    pub connected: bool,
    /// How much of the folder it has, as far as this Device knows (0–100).
    pub completion: f64,
    /// How many files and deletions it still lacks. (Files it ignores count too: Syncthing
    /// doesn't know another Device's ignore patterns.)
    pub need_items: u64,
    /// When this Device last saw it (as Syncthing gives it, an RFC 3339 time), if ever.
    pub last_seen: Option<String>,
}

/// A Sync peer that asked to connect, or offered the Vault's folder, waiting to be accepted.
#[derive(Serialize, Debug, PartialEq, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Waiting {
    pub id: String,
    pub name: String,
    /// It offers the Vault's folder (it already shares it); otherwise it only asked to connect.
    pub offers_folder: bool,
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SyncStatus {
    /// No settings file found, and none set by hand.
    #[serde(rename_all = "camelCase")]
    NotFound { line: String },
    #[serde(rename_all = "camelCase")]
    NotRunning { line: String, address: String },
    #[serde(rename_all = "camelCase")]
    Refused { line: String, address: String },
    #[serde(rename_all = "camelCase")]
    Failed { line: String, reason: String },
    /// Syncthing runs, but no Syncthing folder holds the Vault.
    #[serde(rename_all = "camelCase")]
    NotInFolder { line: String, my_id: String },
    #[serde(rename_all = "camelCase")]
    Folder {
        line: String,
        /// "synced", "syncing", "waiting", "alone" or "problem", for the line's look.
        state: String,
        my_id: String,
        folder_id: String,
        folder_label: String,
        peers: Vec<Peer>,
        waiting: Vec<Waiting>,
    },
}

impl SyncStatus {
    pub fn from_error(error: ApiError, connection: &Connection) -> Self {
        match error {
            ApiError::NotRunning => SyncStatus::NotRunning { line: "Syncthing isn't running".into(), address: connection.address.clone() },
            ApiError::Refused => SyncStatus::Refused { line: "Syncthing didn't accept the API key".into(), address: connection.address.clone() },
            ApiError::Other(reason) => SyncStatus::Failed { line: "Syncthing couldn't be asked".into(), reason },
        }
    }
}

/// `~` and the folder's own separators, as a path to compare with the Vault's.
fn expand(path: &str) -> PathBuf {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    match (path.strip_prefix('~'), home) {
        (Some(rest), Some(home)) => PathBuf::from(home).join(rest.trim_start_matches(['/', '\\'])),
        _ => PathBuf::from(path),
    }
}

fn same_place(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// The Syncthing folder holding the Vault (the Vault itself, or a folder above it): its ID.
/// The deepest one wins if folders are nested.
pub fn folder_holding(folders: &Value, vault: &Path) -> Option<Value> {
    let vault = same_place(vault);
    folders.as_array()?.iter()
        .filter_map(|f| {
            let path = same_place(&expand(f.get("path")?.as_str()?));
            let holds = if cfg!(windows) {
                let lower = |p: &Path| PathBuf::from(p.to_string_lossy().to_lowercase());
                lower(&vault).starts_with(lower(&path))
            } else {
                vault.starts_with(&path)
            };
            holds.then(|| (path.components().count(), f.clone()))
        })
        .max_by_key(|(depth, _)| *depth)
        .map(|(_, f)| f)
}

/// A Sync peer's name: the one Syncthing has for it, else the start of its ID.
fn name_of(devices: &Value, id: &str) -> String {
    devices.as_array()
        .and_then(|all| all.iter().find(|d| d.get("deviceID").and_then(Value::as_str) == Some(id)))
        .and_then(|d| d.get("name").and_then(Value::as_str))
        .filter(|n| !n.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| id.split('-').next().unwrap_or(id).to_string())
}

/// What the status line says, from the folder's state and its Sync peers.
pub fn line_for(folder_state: &str, folder_error: Option<&str>, in_sync: u64, global: u64, peers: &[Peer], waiting: usize) -> (String, &'static str) {
    let percent = |part: f64| format!("{}%", part.clamp(0.0, 100.0).floor() as u32);
    let (mut line, state) = if folder_state == "error" || folder_error.is_some_and(|e| !e.is_empty()) {
        ("Syncthing reports a problem with this folder".to_string(), "problem")
    } else if folder_state.starts_with("sync") {
        let done = if global == 0 { 100.0 } else { in_sync as f64 * 100.0 / global as f64 };
        (format!("Syncing {}", percent(done)), "syncing")
    } else if folder_state.starts_with("scan") || folder_state == "cleaning" {
        ("Checking for changes…".to_string(), "syncing")
    } else if peers.is_empty() {
        ("Not shared with another Device yet".to_string(), "alone")
    } else if let Some(p) = peers.iter().find(|p| p.connected && p.completion < 100.0) {
        (format!("{} is syncing {}", p.name, percent(p.completion)), "syncing")
    } else if let Some(p) = peers.iter().find(|p| !p.connected && p.completion < 100.0) {
        (format!("Waiting for {}", p.name), "waiting")
    } else {
        ("Synced".to_string(), "synced")
    };
    if waiting > 0 {
        line.push_str(&format!(" · {waiting} waiting to be accepted"));
    }
    (line, state)
}

/// Asks Syncthing how the Vault's folder is syncing.
pub fn status(client: &Client, vault: &Path) -> Result<SyncStatus, ApiError> {
    let my_id = client.get("/rest/system/status")?.get("myID").and_then(Value::as_str).unwrap_or("").to_string();
    let folders = client.get("/rest/config/folders")?;
    let Some(folder) = folder_holding(&folders, vault) else {
        return Ok(SyncStatus::NotInFolder { line: "This Vault isn't in a Syncthing folder".into(), my_id });
    };
    let folder_id = folder.get("id").and_then(Value::as_str).unwrap_or("").to_string();
    let label = folder.get("label").and_then(Value::as_str).filter(|l| !l.is_empty()).unwrap_or(&folder_id).to_string();
    let devices = client.get("/rest/config/devices")?;
    let connections = client.get("/rest/system/connections")?;
    let stats = client.get("/rest/stats/device")?;
    let db = client.get(&format!("/rest/db/status?folder={}", param(&folder_id)))?;

    let mut peers = Vec::new();
    for id in folder.get("devices").and_then(Value::as_array).into_iter().flatten().filter_map(|d| d.get("deviceID").and_then(Value::as_str)) {
        if id == my_id {
            continue;
        }
        let answer = client.get(&format!("/rest/db/completion?folder={}&device={}", param(&folder_id), param(id)))?;
        let completion = answer.get("completion").and_then(Value::as_f64).unwrap_or(0.0);
        let need_items = ["needItems", "needDeletes"].iter().filter_map(|k| answer.get(*k).and_then(Value::as_u64)).sum();
        let connected = connections.pointer(&format!("/connections/{id}/connected")).and_then(Value::as_bool).unwrap_or(false);
        let last_seen = stats.pointer(&format!("/{id}/lastSeen")).and_then(Value::as_str)
            .filter(|t| !t.starts_with("1970-")) // never
            .map(str::to_string);
        peers.push(Peer { id: id.to_string(), name: name_of(&devices, id), connected, completion, need_items, last_seen });
    }
    peers.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

    // Devices that asked to connect and aren't known yet, and Sync peers that offer this
    // folder (they share it already; this Device doesn't share it with them yet).
    let pending = client.get("/rest/cluster/pending/devices")?;
    let pending_name = |id: &str| pending.pointer(&format!("/{id}/name")).and_then(Value::as_str).filter(|n| !n.is_empty()).map(str::to_string);
    let mut waiting: Vec<Waiting> = Vec::new();
    let offers = client.get("/rest/cluster/pending/folders")?;
    let key = folder_id.replace('~', "~0").replace('/', "~1");
    if let Some(offer) = offers.pointer(&format!("/{key}/offeredBy")).and_then(Value::as_object) {
        for (id, about) in offer {
            if peers.iter().any(|p| &p.id == id) {
                continue;
            }
            let name = about.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()).map(str::to_string)
                .or_else(|| pending_name(id))
                .unwrap_or_else(|| name_of(&devices, id));
            waiting.push(Waiting { id: id.clone(), name, offers_folder: true });
        }
    }
    for id in pending.as_object().map(|p| p.keys().cloned().collect::<Vec<_>>()).unwrap_or_default() {
        if !waiting.iter().any(|w| w.id == id) {
            let name = pending_name(&id).unwrap_or_else(|| name_of(&Value::Null, &id));
            waiting.push(Waiting { id, name, offers_folder: false });
        }
    }

    let num = |key: &str| db.get(key).and_then(Value::as_u64).unwrap_or(0);
    let (line, state) = line_for(
        db.get("state").and_then(Value::as_str).unwrap_or(""),
        db.get("error").and_then(Value::as_str),
        num("inSyncBytes"),
        num("globalBytes"),
        &peers,
        waiting.len(),
    );
    Ok(SyncStatus::Folder { line, state: state.to_string(), my_id, folder_id, folder_label: label, peers, waiting })
}

// ---- Setting it up ----

/// A Syncthing device ID as Syncthing writes it, or why it isn't one.
fn checked_id(client: &Client, id: &str) -> Result<String, String> {
    let answer = client.get(&format!("/rest/svc/deviceid?id={}", param(id.trim()))).map_err(describe)?;
    match (answer.get("id").and_then(Value::as_str), answer.get("error").and_then(Value::as_str)) {
        (Some(id), _) => Ok(id.to_string()),
        (None, Some(_)) | (None, None) => Err("That isn't a Syncthing device ID. It looks like eight groups of seven letters and digits, such as MFZWI3D-BONSGYC-…, and is shown in Syncthing under Actions → Show ID.".into()),
    }
}

pub fn describe(error: ApiError) -> String {
    match error {
        ApiError::NotRunning => "Syncthing isn't running.".into(),
        ApiError::Refused => "Syncthing didn't accept the API key.".into(),
        ApiError::Other(reason) => format!("Syncthing couldn't do it: {reason}"),
    }
}

/// Shares the Vault's folder with a Sync peer, adding it to Syncthing first if it's new. Used
/// for "Add a Device…" and for accepting one that's waiting.
pub fn share_with(client: &Client, vault: &Path, id: &str, name: &str) -> Result<(), String> {
    let id = checked_id(client, id)?;
    let my_id = client.get("/rest/system/status").map_err(describe)?.get("myID").and_then(Value::as_str).unwrap_or("").to_string();
    if id == my_id {
        return Err("That's this Device's own ID. Use the ID of the Device you want to share the Vault with.".into());
    }
    let folders = client.get("/rest/config/folders").map_err(describe)?;
    let folder = folder_holding(&folders, vault).ok_or("This Vault isn't in a Syncthing folder yet.")?;
    let folder_id = folder.get("id").and_then(Value::as_str).unwrap_or("").to_string();

    let devices = client.get("/rest/config/devices").map_err(describe)?;
    let known = devices.as_array().is_some_and(|all| all.iter().any(|d| d.get("deviceID").and_then(Value::as_str) == Some(id.as_str())));
    if !known {
        // Only the ID and the name: Syncthing fills in its own defaults for the rest.
        client.post("/rest/config/devices", &json!({ "deviceID": id, "name": name.trim() })).map_err(describe)?;
    }
    let mut shared: Vec<Value> = folder.get("devices").and_then(Value::as_array).cloned().unwrap_or_default();
    if !shared.iter().any(|d| d.get("deviceID").and_then(Value::as_str) == Some(id.as_str())) {
        shared.push(json!({ "deviceID": id }));
        client.patch(&format!("/rest/config/folders/{}", param(&folder_id)), &json!({ "devices": shared })).map_err(describe)?;
    }
    Ok(())
}

/// Makes the Vault a Syncthing folder of its own, labelled with its folder's name, so it can be
/// shared. `*-journal` is ignored before Syncthing first looks at it (Phase 0c).
pub fn share_vault(client: &Client, vault: &Path) -> Result<(), String> {
    let folders = client.get("/rest/config/folders").map_err(describe)?;
    if folder_holding(&folders, vault).is_some() {
        return Ok(());
    }
    crate::vault::add_journal_ignore(vault)?;
    let label = vault.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "Vault".into());
    let id = new_folder_id()?;
    let my_id = client.get("/rest/system/status").map_err(describe)?.get("myID").and_then(Value::as_str).unwrap_or("").to_string();
    client
        .post("/rest/config/folders", &json!({ "id": id, "label": label, "path": vault.display().to_string(), "devices": [{ "deviceID": my_id }] }))
        .map_err(describe)?;
    Ok(())
}

/// A folder ID like Syncthing's own ("abcde-fghij").
fn new_folder_id() -> Result<String, String> {
    const LETTERS: &[u8] = b"abcdefghijkmnopqrstuvwxyz234567";
    let mut bytes = [0u8; 10];
    getrandom::fill(&mut bytes).map_err(|e| e.to_string())?;
    let letters: String = bytes.iter().map(|b| LETTERS[*b as usize % LETTERS.len()] as char).collect();
    Ok(format!("{}-{}", &letters[..5], &letters[5..]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    #[test]
    fn reads_the_address_and_key_from_syncthings_settings() {
        let xml = r#"<configuration version="37">
            <folder id="abcde-fghij" path="C:\Sync"></folder>
            <gui enabled="true" tls="false" debugging="false">
                <address>127.0.0.1:8384</address>
                <apikey>k3y</apikey>
                <theme>default</theme>
            </gui>
        </configuration>"#;
        assert_eq!(parse_config(xml), Some(Connection { address: "http://127.0.0.1:8384".into(), api_key: "k3y".into() }));
        let tls = xml.replace(r#"tls="false""#, r#"tls="true""#).replace("127.0.0.1:8384", "0.0.0.0:8385");
        assert_eq!(parse_config(&tls).unwrap().address, "https://127.0.0.1:8385");
        assert_eq!(parse_config(&xml.replace("<apikey>k3y</apikey>", "")), None, "no key: nothing to use");
        assert_eq!(parse_config("not xml"), None);
    }

    #[test]
    fn only_addresses_on_this_device() {
        assert_eq!(local_url("127.0.0.1:8384", false).as_deref(), Some("http://127.0.0.1:8384"));
        assert_eq!(local_url("localhost:8384", true).as_deref(), Some("https://127.0.0.1:8384"));
        assert_eq!(local_url("https://127.0.0.1:8384/", false).as_deref(), Some("https://127.0.0.1:8384"));
        assert_eq!(local_url("[::1]:8384", false).as_deref(), Some("http://[::1]:8384"));
        assert_eq!(local_url(":8384", false).as_deref(), Some("http://127.0.0.1:8384"));
        assert_eq!(local_url("[::]:8384", false).as_deref(), Some("http://127.0.0.1:8384"));
        assert_eq!(local_url("127.0.0.1", false).as_deref(), Some("http://127.0.0.1:8384"));
        for elsewhere in ["192.168.1.20:8384", "nas.local:8384", "unix:///run/st.sock", "/run/st.sock", "http://127.0.0.1:8384/gui", ""] {
            assert_eq!(local_url(elsewhere, false), None, "{elsewhere:?}");
        }
    }

    fn peer(name: &str, connected: bool, completion: f64) -> Peer {
        Peer { id: format!("{name}-ID"), name: name.into(), connected, completion, need_items: 0, last_seen: None }
    }

    #[test]
    fn the_status_line() {
        let line = |state: &str, peers: &[Peer], waiting| line_for(state, None, 630, 1000, peers, waiting);
        assert_eq!(line("idle", &[peer("Phone", true, 100.0)], 0), ("Synced".into(), "synced"));
        assert_eq!(line("syncing", &[peer("Phone", true, 100.0)], 0), ("Syncing 63%".into(), "syncing"));
        assert_eq!(line("scanning", &[], 0).1, "syncing");
        assert_eq!(line("idle", &[], 0), ("Not shared with another Device yet".into(), "alone"));
        assert_eq!(line("idle", &[peer("Laptop", true, 100.0), peer("Phone", false, 80.0)], 0), ("Waiting for Phone".into(), "waiting"));
        assert_eq!(line("idle", &[peer("Phone", true, 40.5)], 0), ("Phone is syncing 40%".into(), "syncing"));
        assert_eq!(line("idle", &[peer("Phone", false, 100.0)], 1), ("Synced · 1 waiting to be accepted".into(), "synced"));
        assert_eq!(line_for("idle", Some("folder path missing"), 0, 0, &[], 0).1, "problem");
    }

    #[test]
    fn finds_the_folder_holding_the_vault() {
        let dir = tempfile::tempdir().unwrap();
        let sync = dir.path().join("Sync");
        let vault = sync.join("Writing");
        std::fs::create_dir_all(&vault).unwrap();
        let folders = json!([
            { "id": "other", "path": dir.path().join("Photos").display().to_string() },
            { "id": "outer", "path": sync.display().to_string() },
            { "id": "inner", "path": format!("{}{}", vault.display(), std::path::MAIN_SEPARATOR) },
        ]);
        assert_eq!(folder_holding(&folders, &vault).unwrap()["id"], "inner", "the deepest one");
        assert_eq!(folder_holding(&json!([folders[1]]), &vault).unwrap()["id"], "outer");
        assert!(folder_holding(&json!([folders[0]]), &vault).is_none());
    }

    /// A pretend Syncthing: answers each request from a table, and remembers what it was sent.
    struct Fake {
        address: String,
        sent: Arc<Mutex<Vec<(String, String, String)>>>,
    }

    fn pretend_syncthing(routes: HashMap<&'static str, Value>, key: &'static str) -> Fake {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let sent = Arc::new(Mutex::new(Vec::new()));
        let log = sent.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut first = String::new();
                if reader.read_line(&mut first).is_err() { continue }
                let (mut length, mut given_key) = (0, String::new());
                loop {
                    let mut header = String::new();
                    reader.read_line(&mut header).unwrap();
                    let header = header.trim_end();
                    if header.is_empty() { break }
                    let (name, value) = header.split_once(':').unwrap();
                    match name.to_ascii_lowercase().as_str() {
                        "content-length" => length = value.trim().parse().unwrap(),
                        "x-api-key" => given_key = value.trim().to_string(),
                        _ => {}
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                let mut parts = first.split_whitespace();
                let (method, target) = (parts.next().unwrap().to_string(), parts.next().unwrap().to_string());
                log.lock().unwrap().push((method.clone(), target.clone(), String::from_utf8(body).unwrap()));
                let route = format!("{method} {}", target);
                let (status, reply) = if given_key != key {
                    ("403 Forbidden", "CSRF Error".to_string())
                } else if let Some(answer) = routes.get(route.as_str()).or_else(|| routes.get(route.split('?').next().unwrap())) {
                    ("200 OK", answer.to_string())
                } else if method != "GET" {
                    ("200 OK", String::new())
                } else {
                    ("404 Not Found", "not found".to_string())
                };
                let mut stream = stream;
                let _ = write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len());
            }
        });
        Fake { address, sent }
    }

    fn routes(vault: &Path) -> HashMap<&'static str, Value> {
        HashMap::from([
            ("GET /rest/system/status", json!({ "myID": "ME" })),
            ("GET /rest/config/folders", json!([{ "id": "wrt-1", "label": "Writing", "path": vault.display().to_string(), "devices": [{ "deviceID": "ME" }, { "deviceID": "PHONE" }] }])),
            ("GET /rest/config/devices", json!([{ "deviceID": "ME", "name": "Laptop" }, { "deviceID": "PHONE", "name": "Pixel" }])),
            ("GET /rest/system/connections", json!({ "connections": { "PHONE": { "connected": false } } })),
            ("GET /rest/stats/device", json!({ "PHONE": { "lastSeen": "2026-11-09T14:02:00+01:00" } })),
            ("GET /rest/db/status", json!({ "state": "idle", "globalBytes": 1000, "inSyncBytes": 1000 })),
            ("GET /rest/db/completion", json!({ "completion": 87.5, "needItems": 3, "needDeletes": 1 })),
            ("GET /rest/cluster/pending/folders", json!({ "wrt-1": { "offeredBy": { "DESKTOP": { "label": "Writing", "name": "" } } } })),
            ("GET /rest/cluster/pending/devices", json!({ "DESKTOP": { "name": "Desktop" }, "STRANGER": { "name": "" } })),
            ("GET /rest/svc/deviceid", json!({ "id": "NEWDEV-OK" })),
        ])
    }

    #[test]
    fn asks_syncthing_how_the_vault_syncs() {
        let dir = tempfile::tempdir().unwrap();
        let fake = pretend_syncthing(routes(dir.path()), "k3y");
        let client = Client::new(&Connection { address: fake.address.clone(), api_key: "k3y".into() });
        let SyncStatus::Folder { line, state, my_id, folder_label, peers, waiting, .. } = status(&client, dir.path()).unwrap() else { panic!("not a folder status") };
        assert_eq!((line.as_str(), state.as_str(), my_id.as_str(), folder_label.as_str()), ("Waiting for Pixel · 2 waiting to be accepted", "waiting", "ME", "Writing"));
        assert_eq!(peers, [Peer { id: "PHONE".into(), name: "Pixel".into(), connected: false, completion: 87.5, need_items: 4, last_seen: Some("2026-11-09T14:02:00+01:00".into()) }]);
        assert_eq!(waiting, [
            Waiting { id: "DESKTOP".into(), name: "Desktop".into(), offers_folder: true },
            Waiting { id: "STRANGER".into(), name: "STRANGER".into(), offers_folder: false },
        ]);
        let sent = fake.sent.lock().unwrap();
        assert!(sent.iter().any(|(_, target, _)| target == "/rest/db/completion?folder=wrt-1&device=PHONE"));
        assert!(sent.iter().all(|(method, _, _)| method == "GET"), "asking never changes anything");
    }

    #[test]
    fn a_wrong_key_or_nothing_running() {
        let dir = tempfile::tempdir().unwrap();
        let fake = pretend_syncthing(routes(dir.path()), "k3y");
        let wrong = Client::new(&Connection { address: fake.address.clone(), api_key: "nope".into() });
        assert_eq!(status(&wrong, dir.path()).unwrap_err(), ApiError::Refused);
        // Nothing listens on a port that was just closed.
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let nobody = Client::new(&Connection { address: format!("http://127.0.0.1:{port}"), api_key: "k3y".into() });
        assert_eq!(status(&nobody, dir.path()).unwrap_err(), ApiError::NotRunning);
    }

    #[test]
    fn a_vault_outside_every_folder() {
        let dir = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let fake = pretend_syncthing(routes(elsewhere.path()), "k3y");
        let client = Client::new(&Connection { address: fake.address.clone(), api_key: "k3y".into() });
        assert!(matches!(status(&client, dir.path()).unwrap(), SyncStatus::NotInFolder { my_id, .. } if my_id == "ME"));
    }

    #[test]
    fn shares_the_vault_with_a_new_sync_peer() {
        let dir = tempfile::tempdir().unwrap();
        let fake = pretend_syncthing(routes(dir.path()), "k3y");
        let client = Client::new(&Connection { address: fake.address.clone(), api_key: "k3y".into() });
        share_with(&client, dir.path(), " newdev ok ", "Desktop").unwrap();
        let sent = fake.sent.lock().unwrap();
        let changes: Vec<_> = sent.iter().filter(|(method, _, _)| method != "GET").collect();
        assert_eq!(changes.len(), 2, "{changes:?}");
        assert_eq!((changes[0].0.as_str(), changes[0].1.as_str()), ("POST", "/rest/config/devices"));
        assert_eq!(serde_json::from_str::<Value>(&changes[0].2).unwrap(), json!({ "deviceID": "NEWDEV-OK", "name": "Desktop" }));
        assert_eq!((changes[1].0.as_str(), changes[1].1.as_str()), ("PATCH", "/rest/config/folders/wrt-1"));
        assert_eq!(serde_json::from_str::<Value>(&changes[1].2).unwrap(), json!({ "devices": [{ "deviceID": "ME" }, { "deviceID": "PHONE" }, { "deviceID": "NEWDEV-OK" }] }));
    }

    #[test]
    fn sharing_with_a_known_peer_only_adds_it_to_the_folder_and_never_twice() {
        let dir = tempfile::tempdir().unwrap();
        let mut table = routes(dir.path());
        table.insert("GET /rest/svc/deviceid", json!({ "id": "PHONE" }));
        let fake = pretend_syncthing(table, "k3y");
        let client = Client::new(&Connection { address: fake.address.clone(), api_key: "k3y".into() });
        share_with(&client, dir.path(), "PHONE", "").unwrap();
        assert!(fake.sent.lock().unwrap().iter().all(|(method, _, _)| method == "GET"), "already shared: nothing to change");

        let mut table = routes(dir.path());
        table.insert("GET /rest/svc/deviceid", json!({ "error": "bad" }));
        let fake = pretend_syncthing(table, "k3y");
        let client = Client::new(&Connection { address: fake.address.clone(), api_key: "k3y".into() });
        assert!(share_with(&client, dir.path(), "nonsense", "").unwrap_err().contains("isn't a Syncthing device ID"));
    }

    /// By hand, against the Syncthing running on this Device: only asks, never changes anything.
    /// `WRTIER_SYNC_VAULT=<a Vault's folder> cargo test -p wrtier_app syncthing_here -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn syncthing_here() {
        let vault = PathBuf::from(std::env::var("WRTIER_SYNC_VAULT").expect("WRTIER_SYNC_VAULT"));
        let connection = find().expect("Syncthing's settings file wasn't found");
        println!("found Syncthing at {}", connection.address);
        let status = status(&Client::new(&connection), &vault).unwrap_or_else(|e| SyncStatus::from_error(e, &connection));
        println!("{}", serde_json::to_string_pretty(&status).unwrap());
    }

    #[test]
    fn makes_the_vault_a_folder_with_journals_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let fake = pretend_syncthing(routes(elsewhere.path()), "k3y");
        let client = Client::new(&Connection { address: fake.address.clone(), api_key: "k3y".into() });
        share_vault(&client, dir.path()).unwrap();
        assert!(std::fs::read_to_string(dir.path().join(".stignore")).unwrap().lines().any(|l| l == "*-journal"));
        let sent = fake.sent.lock().unwrap();
        let (_, target, body) = sent.iter().find(|(method, _, _)| method == "POST").unwrap();
        assert_eq!(target, "/rest/config/folders");
        let body: Value = serde_json::from_str(body).unwrap();
        assert_eq!(body["path"], dir.path().display().to_string());
        assert_eq!(body["devices"], json!([{ "deviceID": "ME" }]));
        assert_eq!(body["id"].as_str().unwrap().len(), 11);
    }
}
