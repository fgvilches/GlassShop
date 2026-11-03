// ─────────────────────────────────────────────────────────────────────────────
// Update detection — v2.
//
// Two things the first version got wrong:
//
// 1. It only knew about titles downloaded *while the list still had them*.
//    Clearing the queue erased the only record. Now every completed download is
//    indexed in `library.json` (keyed by file path) and the list can be cleared
//    without losing the mapping. Files are re-checked on disk, so deleting a
//    file also drops it from the library.
//
// 2. It compared a page's version against itself, which never detects a real
//    update. hShop publishes updates as their own entries whose product code is
//    the base code with the type letter swapped: `CTR-P-AXCE` (game) →
//    `CTR-U-AXCE` (update). Deriving that code and searching for it is enough
//    to find the update entry and read its published version.
// ─────────────────────────────────────────────────────────────────────────────

use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use tauri::{AppHandle, Emitter, Manager as _};

use crate::catalog::{self, SearchQuery};

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

// ── product codes ────────────────────────────────────────────────────────────
// `CTR-P-AXCE`: prefix `CTR`, type `P` (retail) / `N` (VC) / `U` (update),
// four-character game code `AXCE`. Base and update share prefix + game code.

fn split_code(pc: &str) -> Option<(String, String, String)> {
    let parts: Vec<&str> = pc.trim().split('-').collect();
    if parts.len() != 3 {
        return None;
    }
    let prefix = parts[0].trim().to_uppercase();
    let kind = parts[1].trim().to_uppercase();
    let code = parts[2].trim().to_uppercase();
    if prefix.is_empty() || kind.len() != 1 || code.is_empty() {
        return None;
    }
    Some((prefix, kind, code))
}

pub fn is_update_code(pc: &str) -> bool {
    split_code(pc).map(|(_, k, _)| k == "U").unwrap_or(false)
}

/// `CTR-P-AXCE` → `CTR-U-AXCE`. None for updates and malformed codes.
pub fn update_code_for(pc: &str) -> Option<String> {
    let (prefix, kind, code) = split_code(pc)?;
    if kind == "U" {
        return None;
    }
    Some(format!("{prefix}-U-{code}"))
}

/// Same prefix + game code, ignoring the type letter.
fn same_game(a: &str, b: &str) -> bool {
    match (split_code(a), split_code(b)) {
        (Some((pa, _, ca)), Some((pb, _, cb))) => pa == pb && ca == cb,
        _ => false,
    }
}

// ── library ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryEntry {
    pub path: String,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub title_id: String,
    #[serde(default)]
    pub product_code: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub size: Option<i64>,
    #[serde(default)]
    pub completed_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub title_id: String,
    pub name: String,
    /// version of the update already on disk (None = never downloaded)
    pub current_version: Option<String>,
    pub available_version: String,
    pub url: String,
    pub update_title_id: String,
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
    pub path: Option<String>,
    #[serde(default)]
    pub title_id: Option<String>,
    #[serde(default)]
    pub product_code: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub size: Option<i64>,
}

pub struct Checker {
    app: AppHandle,
    library: RwLock<HashMap<String, LibraryEntry>>,
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

        let checker = Arc::new(Self {
            app,
            library: RwLock::new(HashMap::new()),
            state_file: data_dir.join("library.json"),
        });
        checker.load();
        checker
    }

    fn load(&self) {
        // v1 kept `updates-state.json`; still read it so old installs migrate.
        let legacy = self.state_file.with_file_name("updates-state.json");
        for file in [&self.state_file, &legacy] {
            let Ok(raw) = std::fs::read_to_string(file) else { continue };
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else { continue };
            let list = value
                .get("library")
                .or_else(|| value.get("installed"))
                .and_then(|v| v.as_array());
            let Some(list) = list else { continue };
            let mut map = self.library.write().unwrap();
            for item in list {
                if let Ok(entry) = serde_json::from_value::<LibraryEntry>(item.clone()) {
                    if !entry.path.is_empty() {
                        map.entry(entry.path.clone()).or_insert(entry);
                    }
                }
            }
        }
    }

    fn save(&self) {
        let entries: Vec<LibraryEntry> = self.library.read().unwrap().values().cloned().collect();
        let payload = serde_json::json!({ "library": entries });
        if let Ok(text) = serde_json::to_string_pretty(&payload) {
            let _ = std::fs::write(&self.state_file, text);
        }
    }

    /// Called when a download completes.
    pub fn record(&self, payload: RecordPayload) {
        let Some(path) = payload.path.clone().filter(|p| !p.is_empty()) else {
            return;
        };
        let title_id = payload
            .title_id
            .clone()
            .filter(|t| !t.is_empty())
            .or_else(|| Some(payload.id.clone()))
            .unwrap_or_default();

        let entry = LibraryEntry {
            path,
            id: payload.id,
            name: payload.name.unwrap_or_else(|| title_id.clone()),
            title_id,
            product_code: payload.product_code,
            version: payload.version,
            size: payload.size,
            completed_at: now_ms(),
        };
        self.library.write().unwrap().insert(entry.path.clone(), entry);
        self.save();
    }

    pub fn forget(&self, title_id: &str) {
        let mut map = self.library.write().unwrap();
        map.retain(|_, e| e.title_id != title_id && e.id != title_id);
        drop(map);
        self.save();
    }

    /// Entries whose file still exists on disk.
    fn present(&self) -> Vec<LibraryEntry> {
        self.library
            .read()
            .unwrap()
            .values()
            .filter(|e| Path::new(&e.path).exists())
            .cloned()
            .collect()
    }

    fn missing(&self) -> Vec<LibraryEntry> {
        self.library
            .read()
            .unwrap()
            .values()
            .filter(|e| !Path::new(&e.path).exists())
            .cloned()
            .collect()
    }

    /// Files in the download folder that were never indexed (folder carried
    /// over from another machine, or downloaded before this version).
    fn unindexed(&self, dir: &Path) -> Vec<String> {
        let known: Vec<String> = self
            .library
            .read()
            .unwrap()
            .values()
            .map(|e| e.path.clone())
            .collect();
        let Ok(read) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        read.filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_file())
            .filter(|p| {
                matches!(
                    p.extension().and_then(|e| e.to_str()),
                    Some("cia") | Some("3dsx") | Some("app")
                )
            })
            .map(|p| p.to_string_lossy().to_string())
            .filter(|p| !known.iter().any(|k| k == p))
            .collect()
    }

    pub fn snapshot(&self, download_dir: Option<&Path>) -> serde_json::Value {
        let present = self.present();
        let missing = self.missing();
        let unindexed = download_dir.map(|d| self.unindexed(d)).unwrap_or_default();
        serde_json::json!({
            "installed": present,
            "missing": missing,
            "unindexed": unindexed,
        })
    }

    // ── update checks ────────────────────────────────────────────────────────

    pub async fn check_all(&self) -> Vec<UpdateInfo> {
        let mut seen: Vec<String> = Vec::new();
        let mut out = Vec::new();

        for entry in self.present() {
            let Some(pc) = entry.product_code.clone() else { continue };
            if is_update_code(&pc) {
                continue; // base entries drive the check
            }
            if seen.iter().any(|c| same_game(c, &pc)) {
                continue;
            }
            seen.push(pc.clone());
            if let Some(info) = self.check_base(&entry).await {
                out.push(info);
            }
            if seen.len() >= 40 {
                break; // keep the first sweep bounded
            }
        }

        if !out.is_empty() {
            let _ = self.app.emit(
                "updates:available",
                serde_json::json!({ "updates": out }),
            );
        }
        out
    }

    pub async fn check_one(&self, title_id: &str) -> Option<UpdateInfo> {
        let entry = self
            .present()
            .into_iter()
            .find(|e| e.title_id == title_id || e.id == title_id)?;
        let info = self.check_base(&entry).await?;
        let _ = self.app.emit(
            "updates:available",
            serde_json::json!({ "updates": [info.clone()] }),
        );
        Some(info)
    }

    async fn check_base(&self, entry: &LibraryEntry) -> Option<UpdateInfo> {
        let pc = entry.product_code.clone()?;
        let update_code = update_code_for(&pc)?;

        let page = catalog::search_titles(&SearchQuery {
            query: Some(update_code.clone()),
            qt: Some("ProductCode".into()),
            count: Some(10),
            ..Default::default()
        })
        .await
        .ok()?;

        let row = page.titles.iter().find(|t| {
            t.product_code
                .as_deref()
                .map(|c| c.eq_ignore_ascii_case(&update_code))
                .unwrap_or(false)
        })?;

        let detail = catalog::fetch_title_detail(&row.id).await.ok()?;
        let published = parse_version(detail.version.as_deref().unwrap_or(""))?;

        let local = self.present().into_iter().find(|e| {
            e.product_code
                .as_deref()
                .map(|c| c.eq_ignore_ascii_case(&update_code))
                .unwrap_or(false)
        });

        match local {
            Some(local) => {
                let current = parse_version(local.version.as_deref().unwrap_or(""))?;
                if published.cmp_key() > current.cmp_key() {
                    Some(UpdateInfo {
                        title_id: entry.title_id.clone(),
                        name: entry.name.clone(),
                        current_version: Some(current.raw),
                        available_version: published.raw,
                        url: entry_page_url(entry),
                        update_title_id: detail.title_id.clone().unwrap_or(row.id.clone()),
                        product_code: Some(update_code),
                    })
                } else {
                    None
                }
            }
            None => Some(UpdateInfo {
                title_id: entry.title_id.clone(),
                name: entry.name.clone(),
                current_version: None,
                available_version: published.raw,
                url: entry_page_url(entry),
                update_title_id: detail.title_id.clone().unwrap_or(row.id.clone()),
                product_code: Some(update_code),
            }),
        }
    }

    // ── base-first ───────────────────────────────────────────────────────────

    /// Given an update about to be downloaded, find its base game **on disk**
    /// (the download list is irrelevant — the file is what matters). When it is
    /// not on disk, look it up on hShop so the UI can offer to fetch it first.
    pub async fn resolve_base_first(&self, update: &serde_json::Value) -> serde_json::Value {
        let pc = update
            .get("productCode")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if !is_update_code(&pc) {
            return serde_json::json!({
                "base": serde_json::Value::Null,
                "baseTitle": serde_json::Value::Null,
                "update": update
            });
        }

        let base = self.present().into_iter().find(|entry| {
            entry
                .product_code
                .as_deref()
                .map(|c| same_game(c, &pc) && !is_update_code(c))
                .unwrap_or(false)
        });

        let base_title = if base.is_none() {
            self.find_base_title(&pc).await
        } else {
            None
        };

        serde_json::json!({ "base": base, "baseTitle": base_title, "update": update })
    }

    /// `CTR-U-AXCE` → the base entry on hShop (`CTR-P-AXCE` or `CTR-N-AXCE`…).
    fn find_base_title(
        &self,
        update_code: &str,
    ) -> futures_util::future::BoxFuture<'_, Option<catalog::TitleRow>> {
        let code = update_code.to_string();
        Box::pin(async move { find_base_title(&code).await })
    }

    // ── folder scan ──────────────────────────────────────────────────────────

    /// Indexes files that are in the download folder but not in the library:
    /// the filename is searched on hShop and a confident match is stored, so an
    /// carried-over folder also gets update checks.
    pub async fn scan_folder(&self, dir: &Path) -> serde_json::Value {
        let files = self.unindexed(dir);
        let mut matched = 0;
        let mut skipped: Vec<String> = Vec::new();

        for path in files.iter().take(80) {
            let stem = Path::new(path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            let clean = clean_name(&stem);
            if clean.len() < 3 {
                skipped.push(path.clone());
                continue;
            }

            let Ok(page) = catalog::search_titles(&SearchQuery {
                query: Some(clean.clone()),
                qt: Some("Text".into()),
                count: Some(10),
                ..Default::default()
            })
            .await
            else {
                skipped.push(path.clone());
                continue;
            };

            let Some(row) = page
                .titles
                .into_iter()
                .find(|t| names_match(&clean, &t.name))
            else {
                skipped.push(path.clone());
                continue;
            };

            let detail = catalog::fetch_title_detail(&row.id).await.ok();
            self.record(RecordPayload {
                id: row.id.clone(),
                name: Some(row.name.clone()),
                path: Some(path.clone()),
                title_id: row.title_id.clone(),
                product_code: row.product_code.clone(),
                version: detail.and_then(|d| d.version).or_else(|| row.version.clone()),
                size: row.size_bytes,
            });
            matched += 1;
        }

        serde_json::json!({ "matched": matched, "skipped": skipped })
    }
}

/// `CTR-U-AXCE` → the base entry on hShop. The type letter varies by release
/// (retail `P`, virtual console `N`, …), so the plausible ones are tried.
pub async fn find_base_title(update_code: &str) -> Option<catalog::TitleRow> {
    let (prefix, _kind, code) = split_code(update_code)?;
    for kind in ["P", "N", "H", "A", "Q", "C"] {
        let candidate = format!("{prefix}-{kind}-{code}");
        let Ok(page) = catalog::search_titles(&SearchQuery {
            query: Some(candidate.clone()),
            qt: Some("ProductCode".into()),
            count: Some(10),
            ..Default::default()
        })
        .await
        else {
            continue;
        };
        if let Some(row) = page.titles.into_iter().find(|t| {
            t.product_code
                .as_deref()
                .map(|c| c.eq_ignore_ascii_case(&candidate))
                .unwrap_or(false)
        }) {
            return Some(row);
        }
    }
    None
}

/// Lowercase alphanumeric form used to compare filenames with titles.
fn clean_name(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// A filename matches a title when one contains the other (titles on hShop are
/// often a prefix of the local file name, which carries region/version tags).
fn names_match(clean: &str, title: &str) -> bool {
    let t = clean_name(title);
    if t.is_empty() || clean.is_empty() {
        return false;
    }
    if clean.contains(&t) || t.contains(clean) {
        return true;
    }
    // fall back to the first three words
    let head: Vec<&str> = t.split(' ').take(3).collect();
    !head.is_empty() && clean.contains(&head.join(" "))
}

fn entry_page_url(entry: &LibraryEntry) -> String {
    format!("{}/t/{}", catalog::BASE, entry.id)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_update_code() {
        assert_eq!(update_code_for("CTR-P-AXCE").as_deref(), Some("CTR-U-AXCE"));
        assert_eq!(update_code_for("CTR-N-RA5J").as_deref(), Some("CTR-U-RA5J"));
        assert_eq!(update_code_for("CTR-U-AXCE"), None);
    }

    #[test]
    fn detects_update_codes() {
        assert!(is_update_code("CTR-U-AXCE"));
        assert!(!is_update_code("CTR-P-AXCE"));
        assert!(!is_update_code("garbage"));
    }

    #[test]
    fn matches_same_game_across_types() {
        assert!(same_game("CTR-P-AXCE", "CTR-U-AXCE"));
        assert!(!same_game("CTR-P-AXCE", "CTR-U-ZZZZ"));
    }

    #[test]
    fn parses_and_orders_versions() {
        let v = parse_version("34.30.0 (35296)").unwrap();
        assert_eq!(v.cmp_key(), (34, 30, 0, 35296));
        assert!(parse_version("0.0.0 (0)").unwrap().cmp_key() < v.cmp_key());
    }

    #[test]
    fn matches_filenames_to_titles() {
        let file = clean_name("Super Smash Bros. for Nintendo 3DS (USA).cia");
        assert_eq!(file, "super smash bros for nintendo 3ds usa cia");
        assert!(names_match(&file, "Super Smash Bros. for Nintendo 3DS"));

        let zelda = clean_name("The Legend of Zelda - Ocarina of Time 3D");
        assert!(names_match(&zelda, "The Legend of Zelda: Ocarina of Time 3D"));
        assert!(!names_match(&zelda, "Pokemon Ultra Sun"));
    }

    #[test]
    #[ignore]
    fn live_finds_base_of_update() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let row = rt
            .block_on(find_base_title("CTR-U-AXCE"))
            .expect("base title for CTR-U-AXCE");
        println!("base of CTR-U-AXCE → /t/{} {} ({:?})", row.id, row.name, row.product_code);
        assert!(row
            .product_code
            .as_deref()
            .map(|c| c.eq_ignore_ascii_case("CTR-P-AXCE"))
            .unwrap_or(false));
    }
}
