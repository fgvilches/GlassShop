// ─────────────────────────────────────────────────────────────────────────────
// GlassShop — Tauri entry point (port of the Electron main process).
// Liquid glass on macOS comes from NSVisualEffectView via window-vibrancy; the
// renderer keeps painting the same glass-card design language on top.
// ─────────────────────────────────────────────────────────────────────────────

mod catalog;
mod commands;
mod diag;
mod downloads;
mod turnstile;
mod updates;

use serde_json::json;
use std::sync::Arc;
use tauri::menu::{MenuBuilder, MenuItemBuilder, PredefinedMenuItem, SubmenuBuilder};
use tauri::{Emitter, Listener, Manager as _};

pub type SharedChecker = Arc<updates::Checker>;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let handle = app.handle().clone();

            // ── managers ────────────────────────────────────────────────────
            let mgr = downloads::Manager::new(handle.clone());
            let checker = updates::Checker::new(handle.clone());

            // Retry after an expired CDN link re-runs the Turnstile resolver.
            {
                let resolver_handle = handle.clone();
                let resolver: downloads::Resolver = Arc::new(move |id: String| {
                    let h = resolver_handle.clone();
                    Box::pin(async move {
                        let links = turnstile::resolve(&h, &id).await?;
                        links
                            .direct
                            .ok_or_else(|| "no direct link after captcha".to_string())
                    })
                });
                mgr.set_resolver(resolver);
            }

            // When a download finishes, remember the published version so the
            // update checker can spot a newer release later.
            {
                let complete_handle = handle.clone();
                let checker_for_task = checker.clone();
                app.listen("downloads:completed", move |event| {
                    let payload: serde_json::Value =
                        serde_json::from_str(event.payload()).unwrap_or(serde_json::Value::Null);
                    let Some(id) = payload
                        .get("id")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                    else {
                        return;
                    };
                    let dir = payload
                        .get("dir")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let filename = payload
                        .get("filename")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let size = payload.get("size").and_then(|v| v.as_i64());

                    let complete_handle = complete_handle.clone();
                    let checker = checker_for_task.clone();
                    tauri::async_runtime::spawn(async move {
                        // the page carries the product code and version that
                        // update detection needs
                        let detail = catalog::fetch_title_detail(&id).await.ok();
                        let path = if dir.is_empty() || filename.is_empty() {
                            String::new()
                        } else {
                            std::path::Path::new(&dir)
                                .join(&filename)
                                .to_string_lossy()
                                .to_string()
                        };

                        checker.record(updates::RecordPayload {
                            id: id.clone(),
                            name: detail.as_ref().map(|d| d.name.clone()),
                            path: Some(path),
                            title_id: detail.as_ref().and_then(commands::detail_title_id),
                            product_code: detail.as_ref().and_then(|d| d.product_code.clone()),
                            version: detail.as_ref().and_then(|d| d.version.clone()),
                            size,
                        });

                        // and look for an update right away
                        let found = checker.check_one(&id).await;
                        if let Some(info) = found {
                            let _ = complete_handle.emit("updates:available", json!({
                                "updates": [info]
                            }));
                        }
                        let _ = complete_handle.emit("updates:recorded", json!({ "id": id }));
                    });
                });
            }

            app.manage(mgr.clone());
            app.manage(checker.clone());

            // ── background update sweep ─────────────────────────────────────
            // the renderer asks for a check on boot; this keeps it fresh while
            // the app stays open, without the user having to press anything
            {
                let poll_checker = checker.clone();
                let poll_handle = handle.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(25)).await;
                    loop {
                        let found = poll_checker.check_all().await;
                        diag::log(
                            &poll_handle,
                            format!("update sweep: {} update(s) available", found.len()),
                        );
                        tokio::time::sleep(std::time::Duration::from_secs(12 * 60 * 60)).await;
                    }
                });
            }

            // ── window chrome per platform ──────────────────────────────────
            // macOS keeps its traffic lights over an overlay title bar; every
            // other platform goes borderless so the glass fills the window,
            // with the controls drawn by the renderer instead.
            #[cfg(target_os = "macos")]
            {
                use window_vibrancy::{apply_vibrancy, NSVisualEffectMaterial, NSVisualEffectState};
                if let Some(win) = app.get_webview_window("main") {
                    let _ = apply_vibrancy(
                        &win,
                        NSVisualEffectMaterial::UnderWindowBackground,
                        Some(NSVisualEffectState::Active),
                        Some(14.0),
                    );
                }
                build_menu(app, &mgr)?;
            }

            #[cfg(not(target_os = "macos"))]
            {
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.set_decorations(false);
                }
            }

            // the window is created hidden so these adjustments land before it
            // is ever painted
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.show();
                let _ = win.set_focus();
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::catalog_categories,
            commands::catalog_subcategories,
            commands::catalog_titles,
            commands::catalog_search,
            commands::title_detail,
            commands::download_resolve,
            commands::download_add,
            commands::download_pause,
            commands::download_resume,
            commands::download_cancel,
            commands::download_retry,
            commands::download_remove,
            commands::download_reveal,
            commands::download_pause_all,
            commands::download_resume_all,
            commands::download_clear_completed,
            commands::download_settings,
            commands::download_snapshot,
            commands::updates_snapshot,
            commands::updates_check,
            commands::updates_check_one,
            commands::updates_record_installed,
            commands::updates_forget,
            commands::updates_base_first,
            commands::updates_scan,
            commands::updates_extras,
            commands::app_open_external,
            commands::app_pick_dir,
            commands::app_platform,
            commands::window_minimize,
            commands::window_toggle_maximize,
            commands::window_close,
        ])
        .run(tauri::generate_context!())
        .expect("error while running GlassShop");
}

#[cfg(target_os = "macos")]
fn build_menu(app: &tauri::App, mgr: &downloads::Manager) -> tauri::Result<()> {
    let app_handle = app.handle().clone();
    let manager = mgr.clone();

    let prefs = MenuItemBuilder::with_id("prefs", "Preferences…")
        .accelerator("CmdOrCtrl+,")
        .build(app)?;
    let new_search = MenuItemBuilder::with_id("search", "New Search")
        .accelerator("CmdOrCtrl+F")
        .build(app)?;
    let go_downloads = MenuItemBuilder::with_id("go-downloads", "Go to Downloads")
        .accelerator("CmdOrCtrl+J")
        .build(app)?;
    let pause_all = MenuItemBuilder::with_id("pause-all", "Pause All").build(app)?;
    let resume_all = MenuItemBuilder::with_id("resume-all", "Resume All").build(app)?;

    let hide = PredefinedMenuItem::hide(app, Some("Hide GlassShop"))?;
    let quit = PredefinedMenuItem::quit(app, Some("Quit GlassShop"))?;
    let close = PredefinedMenuItem::close_window(app, Some("Close Window"))?;
    let sep_a = PredefinedMenuItem::separator(app)?;
    let sep_b = PredefinedMenuItem::separator(app)?;
    let sep_c = PredefinedMenuItem::separator(app)?;

    let app_menu = SubmenuBuilder::new(app, "GlassShop")
        .item(&prefs)
        .item(&sep_a)
        .item(&hide)
        .item(&quit)
        .build()?;

    let file_menu = SubmenuBuilder::new(app, "File")
        .item(&new_search)
        .item(&sep_b)
        .item(&close)
        .build()?;

    let downloads_menu = SubmenuBuilder::new(app, "Downloads")
        .item(&pause_all)
        .item(&resume_all)
        .item(&sep_c)
        .item(&go_downloads)
        .build()?;

    let menu = MenuBuilder::new(app)
        .items(&[&app_menu, &file_menu, &downloads_menu])
        .build()?;

    app.set_menu(menu)?;

    app.on_menu_event(move |_app, event| {
        let id = event.id().as_ref().to_string();
        match id.as_str() {
            "prefs" => {
                let _ = app_handle.emit("ui:navigate", json!({ "view": "settings" }));
            }
            "search" => {
                let _ = app_handle.emit("ui:navigate", json!({ "view": "search" }));
            }
            "go-downloads" => {
                let _ = app_handle.emit("ui:navigate", json!({ "view": "downloads" }));
            }
            "pause-all" => manager.pause_all(),
            "resume-all" => manager.resume_all(),
            _ => {}
        }
    });

    Ok(())
}
