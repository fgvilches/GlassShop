// ─────────────────────────────────────────────────────────────────────────────
// Tauri commands — the replacement for the Electron IPC surface defined in
// src/main/index.js. Shapes and error convention are preserved: failures come
// back as `{ "error": "…" }` instead of throwing, exactly like the old
// handleErrors() wrapper, so the renderer keeps working unchanged.
// ─────────────────────────────────────────────────────────────────────────────

use serde_json::{json, Value};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::catalog::{self, SearchQuery, TitleDetail, TitlesQuery};
use crate::downloads::{AddInfo, Manager, Snapshot};
use crate::updates::{RecordPayload, UpdateInfo};
use crate::{turnstile, SharedChecker};

fn fail(e: impl ToString) -> Value {
    json!({ "error": e.to_string() })
}

// ── catalog ──────────────────────────────────────────────────────────────────

#[tauri::command]
pub fn catalog_categories() -> Value {
    json!(catalog::categories())
}

#[tauri::command]
pub async fn catalog_subcategories(category_id: String) -> Value {
    match catalog::fetch_subcategories(&category_id).await {
        Ok(list) => json!(list),
        Err(e) => fail(e),
    }
}

#[tauri::command]
pub async fn catalog_titles(opts: TitlesQuery) -> Value {
    match catalog::fetch_titles(&opts).await {
        Ok(page) => json!(page),
        Err(e) => fail(e),
    }
}

#[tauri::command]
pub async fn catalog_search(opts: SearchQuery) -> Value {
    match catalog::search_titles(&opts).await {
        Ok(page) => json!(page),
        Err(e) => fail(e),
    }
}

#[tauri::command]
pub async fn title_detail(id: String) -> Value {
    match catalog::fetch_title_detail(&id).await {
        Ok(detail) => json!(detail),
        Err(e) => fail(e),
    }
}

// ── downloads ────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn download_resolve(app: AppHandle, id: String) -> Value {
    match turnstile::resolve(&app, &id).await {
        Ok(links) => json!(links),
        Err(e) => fail(e),
    }
}

#[tauri::command]
pub fn download_add(manager: State<'_, Manager>, info: AddInfo) -> Value {
    manager.add(info)
}

#[tauri::command]
pub fn download_pause(manager: State<'_, Manager>, id: String) -> Value {
    manager.pause(&id);
    json!({ "ok": true })
}

#[tauri::command]
pub fn download_resume(manager: State<'_, Manager>, id: String) -> Value {
    manager.resume(&id);
    json!({ "ok": true })
}

#[tauri::command]
pub fn download_cancel(manager: State<'_, Manager>, id: String) -> Value {
    manager.cancel(&id);
    json!({ "ok": true })
}

#[tauri::command]
pub async fn download_retry(manager: State<'_, Manager>, id: String) -> Result<Value, String> {
    manager.retry(&id).await?;
    Ok(json!({ "ok": true }))
}

#[tauri::command]
pub fn download_remove(manager: State<'_, Manager>, id: String) -> Value {
    manager.remove(&id);
    json!({ "ok": true })
}

#[tauri::command]
pub fn download_reveal(manager: State<'_, Manager>, id: String) -> Value {
    manager.reveal(&id);
    json!({ "ok": true })
}

#[tauri::command]
pub fn download_pause_all(manager: State<'_, Manager>) -> Value {
    manager.pause_all();
    json!({ "ok": true })
}

#[tauri::command]
pub fn download_resume_all(manager: State<'_, Manager>) -> Value {
    manager.resume_all();
    json!({ "ok": true })
}

#[tauri::command]
pub fn download_clear_completed(manager: State<'_, Manager>) -> Value {
    manager.clear_completed();
    json!({ "ok": true })
}

#[tauri::command]
pub fn download_settings(manager: State<'_, Manager>, patch: Value) -> Value {
    manager.set_settings(patch);
    json!({ "ok": true })
}

#[tauri::command]
pub fn download_snapshot(manager: State<'_, Manager>) -> Snapshot {
    manager.snapshot()
}

// ── updates ──────────────────────────────────────────────────────────────────

#[tauri::command]
pub fn updates_snapshot(checker: State<'_, SharedChecker>, manager: State<'_, Manager>) -> Value {
    let dir = std::path::PathBuf::from(manager.settings().download_dir);
    checker.snapshot(Some(&dir))
}

#[tauri::command]
pub async fn updates_check(checker: State<'_, SharedChecker>) -> Result<Value, String> {
    let list: Vec<UpdateInfo> = checker.check_all().await;
    Ok(json!(list))
}

#[tauri::command]
pub async fn updates_check_one(
    checker: State<'_, SharedChecker>,
    title_id: String,
) -> Result<Value, String> {
    Ok(match checker.check_one(&title_id).await {
        Some(info) => json!(info),
        None => Value::Null,
    })
}

#[tauri::command]
pub fn updates_record_installed(checker: State<'_, SharedChecker>, info: RecordPayload) -> Value {
    checker.record(info);
    json!({ "ok": true })
}

#[tauri::command]
pub fn updates_forget(checker: State<'_, SharedChecker>, title_id: String) -> Value {
    checker.forget(&title_id);
    json!({ "ok": true })
}

#[tauri::command]
pub fn updates_base_first(checker: State<'_, SharedChecker>, update_info: Value) -> Value {
    checker.resolve_base_first(&update_info)
}

// ── app ──────────────────────────────────────────────────────────────────────

#[tauri::command]
pub fn app_open_external(url: String) -> Value {
    let allowed = url.starts_with("https://hshop.erista.me")
        || url.starts_with("https://www.hshop.erista.me")
        || url.starts_with("https://discord.gg");
    if !allowed {
        return fail("blocked url");
    }
    match tauri_plugin_opener::open_url(url, None::<&str>) {
        Ok(()) => json!({ "ok": true }),
        Err(e) => fail(e),
    }
}

#[tauri::command]
pub async fn app_pick_dir(app: AppHandle) -> Value {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_folder(move |folder| {
        let _ = tx.send(folder);
    });
    match rx.await {
        Ok(Some(path)) => json!(path.to_string()),
        _ => Value::Null,
    }
}

// ── helpers exposed to lib.rs ────────────────────────────────────────────────

pub fn detail_title_id(detail: &TitleDetail) -> Option<String> {
    detail.title_id.clone().or_else(|| Some(detail.id.clone()))
}

// ── window chrome (borderless windows on Windows/Linux) ─────────────────────

#[tauri::command]
pub fn app_platform() -> String {
    std::env::consts::OS.to_string()
}

#[tauri::command]
pub fn window_minimize(window: tauri::WebviewWindow) -> Value {
    let _ = window.minimize();
    json!({ "ok": true })
}

#[tauri::command]
pub fn window_toggle_maximize(window: tauri::WebviewWindow) -> Value {
    let maximized = window.is_maximized().unwrap_or(false);
    let _ = if maximized {
        window.unmaximize()
    } else {
        window.maximize()
    };
    json!({ "maximized": !maximized })
}

#[tauri::command]
pub fn window_close(window: tauri::WebviewWindow) -> Value {
    let _ = window.close();
    json!({ "ok": true })
}
