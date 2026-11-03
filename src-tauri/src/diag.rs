// ─────────────────────────────────────────────────────────────────────────────
// Minimal file diagnostics. The verification + download paths run in moments
// the user cannot see, so they leave a breadcrumb trail in
// <app data>/hshop.log when diagnosing a failed download.
// ─────────────────────────────────────────────────────────────────────────────

use std::io::Write;

pub fn log(app: &tauri::AppHandle, msg: impl AsRef<str>) {
    use tauri::Manager as _;
    let Ok(dir) = app.path().app_data_dir() else {
        return;
    };
    let _ = std::fs::create_dir_all(&dir);
    let file = dir.join("hshop.log");
    let Ok(mut handle) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&file)
    else {
        return;
    };
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let _ = writeln!(handle, "[{secs}] {}", msg.as_ref());
}
