// ─────────────────────────────────────────────────────────────────────────────
// Download manager — port of src/main/downloads.js.
// Queue with N parallel workers, HTTP range resume, live speed/ETA, optional
// speed cap, streaming SHA-256 verification, and state persistence.
// ─────────────────────────────────────────────────────────────────────────────

use futures_util::future::BoxFuture;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager as _};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;

use crate::catalog::UA;

pub const QUEUED: &str = "queued";
pub const DOWNLOADING: &str = "downloading";
pub const PAUSED: &str = "paused";
pub const COMPLETED: &str = "completed";
pub const FAILED: &str = "failed";
pub const CANCELLED: &str = "cancelled";

const SAVE_DEBOUNCE: Duration = Duration::from_millis(600);
const BROADCAST_MIN: Duration = Duration::from_millis(300);

/// Resolves a fresh CDN URL for a title (Turnstile-gated). Set after startup.
pub type Resolver = Arc<dyn Fn(String) -> BoxFuture<'static, Result<String, String>> + Send + Sync>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub download_dir: String,
    pub concurrency: usize,
    pub speed_limit: u64,
    pub auto_verify: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            download_dir: String::new(),
            concurrency: 3,
            speed_limit: 0,
            auto_verify: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub id: String,
    pub name: String,
    pub url: String,
    pub filename: String,
    pub dir: String,
    #[serde(default)]
    pub size_bytes: Option<i64>,
    #[serde(default)]
    pub received: i64,
    pub status: String,
    #[serde(default)]
    pub speed: f64,
    #[serde(default)]
    pub eta: f64,
    #[serde(default)]
    pub expected_sha256: Option<String>,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub verified: Option<bool>,
    #[serde(default)]
    pub error: Option<String>,
    pub added_at: i64,
    #[serde(default)]
    pub completed_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub settings: Settings,
    pub active: usize,
    pub items: Vec<Item>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AddInfo {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub filename: Option<String>,
    #[serde(default)]
    pub size_bytes: Option<i64>,
    #[serde(default)]
    pub sha256: Option<String>,
}

pub fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if (c as u32) < 0x20 => '_',
            c => c,
        })
        .collect();
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim();
    if trimmed.is_empty() {
        return "download".to_string();
    }
    trimmed.chars().take(180).collect()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub struct Inner {
    app: AppHandle,
    items: RwLock<Vec<Item>>,
    tokens: AsyncMutex<HashMap<String, CancellationToken>>,
    active: AtomicUsize,
    settings: RwLock<Settings>,
    resolver: RwLock<Option<Resolver>>,
    last_save: RwLock<Instant>,
    last_broadcast: RwLock<Instant>,
    state_file: PathBuf,
}

#[derive(Clone)]
pub struct Manager {
    inner: Arc<Inner>,
}

impl Manager {
    pub fn new(app: AppHandle) -> Self {
        let data_dir = app
            .path()
            .app_data_dir()
            .unwrap_or_else(|_| PathBuf::from("."));
        let _ = std::fs::create_dir_all(&data_dir);
        let state_file = data_dir.join("downloads-state.json");

        let default_dir = app
            .path()
            .download_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("hShop");

        let inner = Arc::new(Inner {
            app,
            items: RwLock::new(Vec::new()),
            tokens: AsyncMutex::new(HashMap::new()),
            active: AtomicUsize::new(0),
            settings: RwLock::new(Settings {
                download_dir: default_dir.to_string_lossy().to_string(),
                ..Default::default()
            }),
            resolver: RwLock::new(None),
            last_save: RwLock::new(Instant::now() - SAVE_DEBOUNCE),
            last_broadcast: RwLock::new(Instant::now() - BROADCAST_MIN),
            state_file,
        });

        let mgr = Self { inner };
        mgr.load();
        mgr
    }

    pub fn set_resolver(&self, resolver: Resolver) {
        if let Ok(mut slot) = self.inner.resolver.write() {
            *slot = Some(resolver);
        }
    }

    // ── persistence ──────────────────────────────────────────────────────────

    fn load(&self) {
        if let Ok(raw) = std::fs::read_to_string(&self.inner.state_file) {
            if let Ok(state) = serde_json::from_str::<serde_json::Value>(&raw) {
                if let Some(settings) = state.get("settings") {
                    if let Ok(s) = serde_json::from_value::<Settings>(settings.clone()) {
                        if let Ok(mut cur) = self.inner.settings.write() {
                            let fallback = cur.download_dir.clone();
                            *cur = s;
                            if cur.download_dir.is_empty() {
                                cur.download_dir = fallback;
                            }
                        }
                    }
                }
                if let Some(items) = state.get("items").and_then(|v| v.as_array()) {
                    if let Ok(mut list) = self.inner.items.write() {
                        for v in items {
                            if let Ok(mut it) = serde_json::from_value::<Item>(v.clone()) {
                                // never resume as active after a restart
                                if it.status == DOWNLOADING || it.status == QUEUED {
                                    it.status = PAUSED.to_string();
                                }
                                it.speed = 0.0;
                                it.eta = 0.0;
                                list.push(it);
                            }
                        }
                    }
                }
            }
        }
        let dir = self.settings().download_dir;
        let _ = std::fs::create_dir_all(dir);
    }

    fn save(&self, force: bool) {
        let should = {
            let last = self.inner.last_save.read().unwrap();
            force || last.elapsed() >= SAVE_DEBOUNCE
        };
        if !should {
            return;
        }
        if let Ok(mut last) = self.inner.last_save.write() {
            *last = Instant::now();
        }
        let settings = self.settings();
        let mut items: Vec<Item> = self.inner.items.read().unwrap().clone();
        for it in items.iter_mut() {
            it.speed = 0.0;
            it.eta = 0.0;
        }
        let payload = serde_json::json!({ "settings": settings, "items": items });
        if let Ok(text) = serde_json::to_string_pretty(&payload) {
            let _ = std::fs::write(&self.inner.state_file, text);
        }
    }

    // ── accessors ────────────────────────────────────────────────────────────

    pub fn settings(&self) -> Settings {
        self.inner.settings.read().unwrap().clone()
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            settings: self.settings(),
            active: self.inner.active.load(Ordering::SeqCst),
            items: self.inner.items.read().unwrap().clone(),
        }
    }

    pub fn push(&self, force: bool) {
        if !force {
            let last = self.inner.last_broadcast.read().unwrap();
            if last.elapsed() < BROADCAST_MIN {
                return;
            }
        }
        if let Ok(mut last) = self.inner.last_broadcast.write() {
            *last = Instant::now();
        }
        let _ = self.inner.app.emit("downloads:snapshot", self.snapshot());
    }

    fn item_mut<R>(&self, id: &str, f: impl FnOnce(&mut Item) -> R) -> Option<R> {
        let mut items = self.inner.items.write().unwrap();
        items.iter_mut().find(|it| it.id == id).map(f)
    }

    fn status_of(&self, id: &str) -> Option<String> {
        self.inner
            .items
            .read()
            .unwrap()
            .iter()
            .find(|it| it.id == id)
            .map(|it| it.status.clone())
    }

    // ── queue control ────────────────────────────────────────────────────────

    pub fn add(&self, info: AddInfo) -> serde_json::Value {
        let id = info.id.clone();
        let existing = {
            let items = self.inner.items.read().unwrap();
            items.iter().find(|it| it.id == id).cloned()
        };
        if let Some(it) = existing {
            match it.status.as_str() {
                COMPLETED => return serde_json::json!({"ok": true, "existed": "completed"}),
                DOWNLOADING | QUEUED => {
                    return serde_json::json!({"ok": true, "existed": "active"})
                }
                _ => {
                    self.item_mut(&id, |it| {
                        it.status = QUEUED.to_string();
                        it.error = None;
                    });
                    self.save(true);
                    self.push(true);
                    self.pump();
                    return serde_json::json!({"ok": true, "existed": "requeued"});
                }
            }
        }

        let dir = self.settings().download_dir;
        let name = info.name.clone().unwrap_or_else(|| format!("hShop title {id}"));
        let raw_name = info
            .filename
            .clone()
            .unwrap_or_else(|| format!("{}.cia", info.name.clone().unwrap_or_else(|| id.clone())));
        let filename = sanitize_filename(&raw_name);

        let item = Item {
            id: id.clone(),
            name,
            url: info.url,
            filename,
            dir,
            size_bytes: info.size_bytes,
            received: 0,
            status: QUEUED.to_string(),
            speed: 0.0,
            eta: 0.0,
            expected_sha256: info.sha256,
            sha256: None,
            verified: None,
            error: None,
            added_at: now_ms(),
            completed_at: None,
        };
        self.inner.items.write().unwrap().push(item);
        self.save(true);
        self.push(true);
        self.pump();
        serde_json::json!({"ok": true, "existed": false})
    }

    pub fn pump(&self) {
        let concurrency = self.settings().concurrency.max(1);
        loop {
            if self.inner.active.load(Ordering::SeqCst) >= concurrency {
                break;
            }
            let next_id = {
                let items = self.inner.items.read().unwrap();
                items.iter().find(|it| it.status == QUEUED).map(|it| it.id.clone())
            };
            let Some(id) = next_id else { break };
            let mgr = self.clone();
            let run_id = id.clone();
            tauri::async_runtime::spawn(async move {
                mgr.run(run_id).await;
            });
            // mark as downloading synchronously so the loop does not re-pick it
            self.item_mut(&id, |it| it.status = DOWNLOADING.to_string());
            self.inner.active.fetch_add(1, Ordering::SeqCst);
        }
    }

    pub fn pause(&self, id: &str) {
        let status = self.status_of(id);
        match status.as_deref() {
            Some(DOWNLOADING) => {
                let id = id.to_string();
                let inner = self.inner.clone();
                tauri::async_runtime::spawn(async move {
                    let tokens = inner.tokens.lock().await;
                    if let Some(tok) = tokens.get(&id) {
                        tok.cancel();
                    }
                });
            }
            Some(QUEUED) => {
                self.item_mut(id, |it| it.status = PAUSED.to_string());
                self.save(true);
                self.push(true);
            }
            _ => {}
        }
    }

    pub fn resume(&self, id: &str) {
        let should = matches!(
            self.status_of(id).as_deref(),
            Some(PAUSED) | Some(FAILED) | Some(CANCELLED)
        );
        if should {
            self.item_mut(id, |it| {
                it.status = QUEUED.to_string();
                it.error = None;
            });
            self.save(true);
            self.push(true);
            self.pump();
        }
    }

    pub fn cancel(&self, id: &str) {
        let status = self.status_of(id).unwrap_or_default();
        if status == DOWNLOADING {
            self.item_mut(id, |it| it.status = CANCELLED.to_string());
            let id = id.to_string();
            let inner = self.inner.clone();
            tauri::async_runtime::spawn(async move {
                let tokens = inner.tokens.lock().await;
                if let Some(tok) = tokens.get(&id) {
                    tok.cancel();
                }
            });
        } else if status != COMPLETED {
            self.item_mut(id, |it| it.status = CANCELLED.to_string());
        }
        self.save(true);
        self.push(true);
    }

    pub fn remove(&self, id: &str) {
        let part = self
            .inner
            .items
            .read()
            .unwrap()
            .iter()
            .find(|it| it.id == id)
            .map(|it| part_path(it));
        if let Some(part) = part {
            let _ = std::fs::remove_file(part);
        }
        {
            let mut items = self.inner.items.write().unwrap();
            items.retain(|it| it.id != id);
        }
        self.save(true);
        self.push(true);
    }

    pub fn retry(&self, id: &str) -> BoxFuture<'static, Result<(), String>> {
        let mgr = self.clone();
        let id_owned = id.to_string();
        Box::pin(async move {
            let (error, url) = {
                let items = mgr.inner.items.read().unwrap();
                match items.iter().find(|it| it.id == id_owned) {
                    Some(it) => (it.error.clone(), it.url.clone()),
                    None => return Err("not found".to_string()),
                }
            };
            let needs_resolve =
                error.as_deref() == Some("link-expired") || url.trim().is_empty();

            if needs_resolve {
                let resolver = {
                    let slot = mgr.inner.resolver.read().unwrap();
                    slot.clone()
                };
                let Some(resolver) = resolver else {
                    return Err("no resolver configured".to_string());
                };
                match resolver(id_owned.clone()).await {
                    Ok(fresh) => {
                        mgr.item_mut(&id_owned, |it| {
                            it.url = fresh;
                            it.error = None;
                        });
                    }
                    Err(e) => {
                        mgr.item_mut(&id_owned, |it| {
                            it.status = FAILED.to_string();
                            it.error = Some(format!("re-resolve failed: {e}"));
                        });
                        mgr.save(true);
                        mgr.push(true);
                        return Err(e);
                    }
                }
            }
            mgr.resume(&id_owned);
            Ok(())
        })
    }

    pub fn pause_all(&self) {
        let ids: Vec<String> = self
            .inner
            .items
            .read()
            .unwrap()
            .iter()
            .map(|it| it.id.clone())
            .collect();
        for id in ids {
            self.pause(&id);
        }
    }

    pub fn resume_all(&self) {
        let ids: Vec<String> = self
            .inner
            .items
            .read()
            .unwrap()
            .iter()
            .map(|it| it.id.clone())
            .collect();
        for id in ids {
            self.resume(&id);
        }
    }

    pub fn clear_completed(&self) {
        {
            let mut items = self.inner.items.write().unwrap();
            items.retain(|it| it.status != COMPLETED);
        }
        self.save(true);
        self.push(true);
    }

    pub fn set_settings(&self, patch: serde_json::Value) {
        {
            let mut s = self.inner.settings.write().unwrap();
            if let Some(v) = patch.get("downloadDir").and_then(|v| v.as_str()) {
                s.download_dir = v.to_string();
                let _ = std::fs::create_dir_all(&s.download_dir);
            }
            if let Some(v) = patch.get("concurrency").and_then(|v| v.as_u64()) {
                s.concurrency = (v as usize).clamp(1, 16);
            }
            if let Some(v) = patch.get("speedLimit").and_then(|v| v.as_u64()) {
                s.speed_limit = v;
            }
            if let Some(v) = patch.get("autoVerify").and_then(|v| v.as_bool()) {
                s.auto_verify = v;
            }
            let dir = s.download_dir.clone();
            let _ = std::fs::create_dir_all(dir);
        }
        self.save(true);
        self.push(true);
        self.pump();
    }

    pub fn reveal(&self, id: &str) {
        let target = {
            let items = self.inner.items.read().unwrap();
            items
                .iter()
                .find(|it| it.id == id)
                .map(|it| (final_path(it), PathBuf::from(&it.dir)))
        };
        if let Some((file, dir)) = target {
            if file.exists() {
                let _ = tauri_plugin_opener::reveal_item_in_dir(&file);
            } else {
                let _ = tauri_plugin_opener::open_path(dir.to_string_lossy().to_string(), None::<&str>);
            }
        }
    }

    // ── worker ───────────────────────────────────────────────────────────────

    async fn run(&self, id: String) {
        let Some(item) = self
            .inner
            .items
            .read()
            .unwrap()
            .iter()
            .find(|it| it.id == id)
            .cloned()
        else {
            self.inner.active.fetch_sub(1, Ordering::SeqCst);
            return;
        };

        let token = CancellationToken::new();
        {
            let mut tokens = self.inner.tokens.lock().await;
            tokens.insert(id.clone(), token.clone());
        }

        let settings = self.settings();
        let part = part_path(&item);
        let final_file = final_path(&item);

        crate::diag::log(
            &self.inner.app,
            format!("download {id}: start {}", item.url.chars().take(80).collect::<String>()),
        );

        let result = self
            .transfer(&id, &item, &part, &settings, &token)
            .await;

        match result {
            Ok((received, hash)) => {
                if let Some(hash) = hash {
                    self.item_mut(&id, |it| {
                        if let Some(expected) = it.expected_sha256.clone() {
                            it.verified = Some(hash.eq_ignore_ascii_case(&expected));
                        }
                        it.sha256 = Some(hash);
                    });
                }
                if let Err(e) = std::fs::rename(&part, &final_file) {
                    self.item_mut(&id, |it| {
                        it.status = FAILED.to_string();
                        it.error = Some(format!("rename failed: {e}"));
                    });
                } else {
                    self.item_mut(&id, |it| {
                        it.status = COMPLETED.to_string();
                        it.received = received;
                        it.speed = 0.0;
                        it.eta = 0.0;
                        it.completed_at = Some(now_ms());
                    });
                }
                self.save(true);
                self.push(true);
                crate::diag::log(
                    &self.inner.app,
                    format!("download {id}: finished, {received} bytes"),
                );
                // richer payload: the update checker indexes the finished file
                let payload = {
                    let items = self.inner.items.read().unwrap();
                    items
                        .iter()
                        .find(|it| it.id == id)
                        .map(|it| {
                            serde_json::json!({
                                "id": it.id,
                                "name": it.name,
                                "filename": it.filename,
                                "dir": it.dir,
                                "size": it.received,
                                "sha256": it.sha256,
                            })
                        })
                };
                if let Some(payload) = payload {
                    let _ = self.inner.app.emit("downloads:completed", payload);
                }
            }
            Err(e) => {
                crate::diag::log(&self.inner.app, format!("download {id}: failed — {e}"));
                let cancelled = token.is_cancelled();
                let status = self.status_of(&id).unwrap_or_default();
                self.item_mut(&id, |it| {
                    it.speed = 0.0;
                    it.eta = 0.0;
                    if cancelled && status != CANCELLED {
                        it.status = PAUSED.to_string();
                    } else if status != CANCELLED {
                        it.status = FAILED.to_string();
                        it.error = Some(e);
                    }
                });
                self.save(true);
                self.push(true);
            }
        }

        {
            let mut tokens = self.inner.tokens.lock().await;
            tokens.remove(&id);
        }
        self.inner.active.fetch_sub(1, Ordering::SeqCst);
        self.pump();
        self.push(true);
    }

    /// Streams bytes to the .part file. Returns (bytes received, sha256) on success.
    async fn transfer(
        &self,
        id: &str,
        item: &Item,
        part: &Path,
        settings: &Settings,
        token: &CancellationToken,
    ) -> Result<(i64, Option<String>), String> {
        if let Some(parent) = part.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("mkdir: {e}"))?;
        }

        // resume offset
        let mut start_at: i64 = 0;
        if let Ok(meta) = tokio::fs::metadata(part).await {
            let have = meta.len() as i64;
            let total = item.size_bytes.unwrap_or(0);
            if have > 0 && (total == 0 || have < total) {
                start_at = have;
            } else {
                let _ = tokio::fs::remove_file(part).await;
            }
        }
        self.item_mut(id, |it| it.received = start_at);

        let mut req = crate::catalog::client()
            .get(&item.url)
            .header(reqwest::header::ACCEPT, "*/*")
            .header(reqwest::header::USER_AGENT, UA);
        if start_at > 0 {
            req = req.header(reqwest::header::RANGE, format!("bytes={start_at}-"));
        }

        let res = tokio::select! {
            r = req.send() => r.map_err(|e| format!("network: {e}"))?,
            _ = token.cancelled() => return Err("aborted".to_string()),
        };

        let status = res.status();
        if matches!(status.as_u16(), 401 | 403 | 404 | 410) {
            return Err("link-expired".to_string());
        }
        let is_partial = status.as_u16() == 206;
        if status.as_u16() != 200 && !is_partial {
            return Err(format!("HTTP {}", status.as_u16()));
        }

        let mut append = is_partial;
        if !is_partial && start_at > 0 {
            // server ignored Range → start over
            start_at = 0;
            append = false;
            self.item_mut(id, |it| it.received = 0);
        }

        let content_len = res
            .headers()
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(0);
        let total = if append && content_len > 0 {
            start_at + content_len
        } else if content_len > 0 {
            content_len
        } else {
            item.size_bytes.unwrap_or(0)
        };
        if total > 0 {
            self.item_mut(id, |it| {
                it.size_bytes = Some(match it.size_bytes {
                    Some(cur) if cur > total => cur,
                    _ => total,
                });
            });
        }

        // Content-Disposition may carry the real filename
        if let Some(cd) = res
            .headers()
            .get(reqwest::header::CONTENT_DISPOSITION)
            .and_then(|v| v.to_str().ok())
        {
            if let Some(name) = parse_content_disposition(cd) {
                let clean = sanitize_filename(&name);
                if !clean.is_empty() {
                    self.item_mut(id, |it| it.filename = clean);
                }
            }
        }

        let mut hasher = Sha256::new();
        if append && settings.auto_verify {
            if let Ok(mut f) = tokio::fs::File::open(part).await {
                use tokio::io::AsyncReadExt;
                let mut buf = vec![0u8; 1 << 16];
                let mut left = start_at as u64;
                while left > 0 {
                    let want = (buf.len() as u64).min(left) as usize;
                    let n = f.read(&mut buf[..want]).await.map_err(|e| e.to_string())?;
                    if n == 0 {
                        break;
                    }
                    hasher.update(&buf[..n]);
                    left -= n as u64;
                }
            }
        }

        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(append)
            .truncate(!append)
            .open(part)
            .await
            .map_err(|e| format!("open: {e}"))?;

        let mut received = start_at;
        let mut stream = res.bytes_stream();
        let mut window: Vec<(Instant, usize)> = Vec::new();
        let mut last_broadcast = Instant::now() - BROADCAST_MIN;
        let mut last_limit = Instant::now();

        loop {
            let chunk = tokio::select! {
                c = stream.next() => c,
                _ = token.cancelled() => {
                    let _ = file.flush().await;
                    return Err("aborted".to_string());
                }
            };
            let Some(chunk) = chunk else { break };
            let bytes = chunk.map_err(|e| format!("stream: {e}"))?;
            file.write_all(&bytes).await.map_err(|e| format!("write: {e}"))?;
            received += bytes.len() as i64;
            if settings.auto_verify {
                hasher.update(&bytes);
            }
            self.item_mut(id, |it| it.received = received);

            // speed window (1.5 s)
            let now = Instant::now();
            window.push((now, bytes.len()));
            while let Some((t, _)) = window.first() {
                if now.duration_since(*t) > Duration::from_millis(1500) {
                    window.remove(0);
                } else {
                    break;
                }
            }

            // optional speed cap
            if settings.speed_limit > 0 {
                let expected = Duration::from_secs_f64(bytes.len() as f64 / settings.speed_limit as f64);
                let spent = now.duration_since(last_limit);
                if spent < expected {
                    tokio::time::sleep(expected - spent).await;
                }
                last_limit = Instant::now();
            }

            if last_broadcast.elapsed() >= BROADCAST_MIN {
                last_broadcast = Instant::now();
                let recent: usize = window.iter().map(|(_, b)| *b).sum();
                let span = window
                    .first()
                    .map(|(t, _)| now.duration_since(*t).as_secs_f64())
                    .unwrap_or(0.0);
                let speed = if span > 0.25 { recent as f64 / span } else { 0.0 };
                let size = self
                    .inner
                    .items
                    .read()
                    .unwrap()
                    .iter()
                    .find(|it| it.id == id)
                    .and_then(|it| it.size_bytes);
                let eta = match (speed, size) {
                    (s, Some(total)) if s > 0.0 => ((total - received) as f64 / s).max(0.0),
                    _ => 0.0,
                };
                self.item_mut(id, |it| {
                    it.speed = speed;
                    it.eta = eta;
                });
                self.push(false);
            }
        }

        file.flush().await.map_err(|e| format!("flush: {e}"))?;
        file.sync_all().await.ok();
        drop(file);

        let hash = if settings.auto_verify {
            Some(hex::encode(hasher.finalize()))
        } else {
            None
        };
        Ok((received, hash))
    }
}

fn part_path(it: &Item) -> PathBuf {
    Path::new(&it.dir).join(format!("{}.part", it.filename))
}

fn final_path(it: &Item) -> PathBuf {
    Path::new(&it.dir).join(&it.filename)
}

fn parse_content_disposition(cd: &str) -> Option<String> {
    let lower = cd.to_ascii_lowercase();
    let key = "filename=";
    let pos = lower.find(key)?;
    let raw = &cd[pos + key.len()..];
    let raw = raw.trim_start_matches('"').trim_start_matches("UTF-8''");
    let end = raw.find(['"', ';']).unwrap_or(raw.len());
    let value = raw[..end].trim();
    if value.is_empty() {
        None
    } else {
        Some(percent_decode(value))
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}
