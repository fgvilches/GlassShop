// ─────────────────────────────────────────────────────────────────────────────
// Update detection — port of src/main/updates.js.
// Compares the version published on hShop against the version already
// downloaded: (a) notifies when a title has an update, (b) detects an update
// being downloaded without its base game.
// ─────────────────────────────────────────────────────────────────────────────

use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tauri::{AppHandle, Emitter, Manager as _};

static VERSION_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(\d+)\.(\d+)\.(\d+)\s*(?:\((\d+)\))?").unwrap());

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    pub build: u32,
    pub raw: String,
}

impl Version {
    pub fn cmp_key(&self) -> (u32, u32, u32, u32) {
        (self.major, self.minor, self.patch, self.build)
    }
}

pub fn parse_version(value: &str) -> Option<Version> {
    let caps = VERSION_RE.captures(value)?;
    Some(Version {
        major: caps.get(1)?.as_str().parse().ok()?,
        minor: caps.get(2)?.as_str().parse().ok()?,
        patch: caps.get(3)?.as_str().parse().ok()?,
        build: caps.get(4).and_then(|m| m.as_str().parse().ok()).unwrap_or(0),
        raw: value.to_string(),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Installed {
    pub id: String,
    pub name: String,
    pub title_id: String,
    #[serde(default)]
    pub product_code: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub downloaded_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub title_id: String,
    pub name: String,
    pub current_version: String,
    pub available_version: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product_code: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RecordPayload {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub title_id: Option<String>,
    #[serde(default)]
    pub product_code: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
}

pub struct Checker {
    app: AppHandle,
    installed: RwLock<HashMap<String, Installed>>,
    state_file: PathBuf,
}

pub type Shared = Arc<Checker>;

impl Checker {
    pub fn new(app: AppHandle) -> Shared {
        let data_dir = app
            .path()
            .app_data_dir()
            .unwrap_or_else(|_| PathBuf::from("."));
        let _ = std::fs::create_dir_all(&data_dir);
        let state_file = data_dir.join("updates-state.json");

        let checker = Arc::new(Self {
            app,
            installed: RwLock::new(HashMap::new()),
            state_file,
        });
        checker.load();
        checker
    }

    fn load(&self) {
        if let Ok(raw) = std::fs::read_to_string(&self.state_file) {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) {
                if let Some(list) = value.get("installed").and_then(|v| v.as_array()) {
                    let mut map = self.installed.write().unwrap();
                    for entry in list {
                        if let Ok(item) = serde_json::from_value::<Installed>(entry.clone()) {
                            map.insert(item.title_id.clone(), item);
                        }
                    }
                }
            }
        }
    }

    fn save(&self) {
        let installed: Vec<Installed> = self.installed.read().unwrap().values().cloned().collect();
        let payload = serde_json::json!({ "installed": installed });
        if let Ok(text) = serde_json::to_string_pretty(&payload) {
            let _ = std::fs::write(&self.state_file, text);
        }
    }

    pub fn record_installed(&self, payload: RecordPayload) {
        let title_id = payload
            .title_id
            .clone()
            .or_else(|| if payload.id.is_empty() { None } else { Some(payload.id.clone()) })
            .unwrap_or_default();
        if title_id.is_empty() {
            return;
        }
        let item = Installed {
            id: if payload.id.is_empty() {
                title_id.clone()
            } else {
                payload.id.clone()
            },
            name: payload.name.unwrap_or_else(|| title_id.clone()),
            title_id: title_id.clone(),
            product_code: payload.product_code,
            version: payload.version,
            downloaded_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0),
        };
        self.installed.write().unwrap().insert(title_id, item);
        self.save();
    }

    pub fn forget(&self, title_id: &str) {
        self.installed.write().unwrap().remove(title_id);
        self.save();
    }

    pub fn snapshot(&self) -> serde_json::Value {
        let list: Vec<Installed> = self.installed.read().unwrap().values().cloned().collect();
        serde_json::json!({ "installed": list })
    }

    fn entries(&self) -> Vec<Installed> {
        self.installed.read().unwrap().values().cloned().collect()
    }

    pub async fn check_all(&self) -> Vec<UpdateInfo> {
        let mut out = Vec::new();
        for entry in self.entries() {
            if let Some(info) = self.check_entry(&entry).await {
                out.push(info);
            }
        }
        if !out.is_empty() {
            let _ = self
                .app
                .emit("updates:available", serde_json::json!({ "updates": out }));
        }
        out
    }

    pub async fn check_one(&self, title_id: &str) -> Option<UpdateInfo> {
        let entry = self.installed.read().unwrap().get(title_id).cloned()?;
        let info = self.check_entry(&entry).await?;
        let _ = self.app.emit(
            "updates:available",
            serde_json::json!({ "updates": [info.clone()] }),
        );
        Some(info)
    }

    async fn check_entry(&self, entry: &Installed) -> Option<UpdateInfo> {
        let detail = crate::catalog::fetch_title_detail(&entry.title_id).await.ok()?;
        let current = parse_version(entry.version.as_deref().unwrap_or(""))?;
        let available = parse_version(detail.version.as_deref().unwrap_or(""))?;
        if available.cmp_key() > current.cmp_key() {
            Some(UpdateInfo {
                title_id: entry.title_id.clone(),
                name: entry.name.clone(),
                current_version: current.raw,
                available_version: available.raw,
                url: detail.url,
                product_code: detail.product_code,
            })
        } else {
            None
        }
    }

    /// When downloading an update, find the base game in the installed list.
    pub fn resolve_base_first(&self, update: &serde_json::Value) -> serde_json::Value {
        let update_pc = update
            .get("productCode")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let base = self.entries().into_iter().find(|entry| {
            let pc = entry.product_code.clone().unwrap_or_default();
            if pc.is_empty() || update_pc.is_empty() {
                return false;
            }
            // same family: "CTR-P-BYSZ" vs "CTR-P-BYSZ-01"
            let a: String = pc.chars().take(pc.len().saturating_sub(2)).collect();
            let b: String = update_pc
                .chars()
                .take(update_pc.len().saturating_sub(2))
                .collect();
            a == b
        });

        serde_json::json!({ "base": base, "update": update })
    }
}
