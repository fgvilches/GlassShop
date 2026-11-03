// ─────────────────────────────────────────────────────────────────────────────
// Turnstile resolver — port of src/main/turnstile.js.
//
// hShop gates downloads behind Cloudflare Turnstile. We NEVER bypass it: a real
// webview loads the title page and Turnstile runs its checks. Like the Electron
// build, the window starts HIDDEN so a non-interactive pass is invisible to the
// user — you press Download and the transfer just starts. If it hasn't resolved
// after a few seconds the window is revealed so the user can tap the checkbox.
//
// Once the page renders the "Direct Download" anchor, the injected watcher
// hands the URL back by navigating to /__hsg_solved?u=…, which is intercepted
// in `on_navigation` — no cross-origin IPC needed. Resolved links are cached
// per title for 30 minutes.
// ─────────────────────────────────────────────────────────────────────────────

use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager as _, WebviewUrl, WebviewWindowBuilder};

use crate::catalog::{DownloadLinks, BASE};

const CACHE_TTL: Duration = Duration::from_secs(30 * 60);
const SOLVE_TIMEOUT: Duration = Duration::from_secs(180);
const REVEAL_AFTER: Duration = Duration::from_secs(6);

static CACHE: Lazy<Mutex<HashMap<String, (DownloadLinks, Instant)>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Watches the page for the post-Turnstile "Direct Download" anchor and reports
/// it back through a same-origin navigation that Rust can intercept.
const WATCHER_JS: &str = r#"
(function () {
  var started = Date.now();
  var done = false;
  function report(href) {
    if (done) return;
    done = true;
    try { location.href = '/__hsg_solved?u=' + encodeURIComponent(href); } catch (e) {}
  }
  function grab() {
    if (done) return;
    var anchors = document.querySelectorAll('a');
    for (var i = 0; i < anchors.length; i++) {
      var a = anchors[i];
      var label = (a.textContent || '').trim();
      var href = a.getAttribute('href') || '';
      if (/direct download/i.test(label) && /^https?:\/\//i.test(href)) {
        report(href);
        return;
      }
    }
    if (Date.now() - started > 175000) done = true;
  }
  var timer = setInterval(grab, 500);
  document.addEventListener('DOMContentLoaded', grab);
  window.addEventListener('load', grab);
})();
"#;

pub fn cached(id: &str) -> Option<DownloadLinks> {
    let map = CACHE.lock().ok()?;
    let (links, ts) = map.get(id)?;
    if ts.elapsed() < CACHE_TTL {
        Some(links.clone())
    } else {
        None
    }
}

/// Resolves the direct CDN URL for `id`. The verification window stays hidden
/// while the silent Turnstile pass can still succeed.
pub async fn resolve(app: &AppHandle, id: &str) -> Result<DownloadLinks, String> {
    if let Some(hit) = cached(id) {
        return Ok(hit);
    }

    let label = format!("turnstile-{id}-{}", nanos_id());
    let page: tauri::Url = format!("{BASE}/t/{id}")
        .parse()
        .map_err(|e| format!("bad url: {e}"))?;

    crate::diag::log(app, format!("turnstile {id}: opening hidden verifier"));

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let nav_tx = tx.clone();

    let mut builder = WebviewWindowBuilder::new(app, &label, WebviewUrl::External(page))
        .title("Verificando — GlassShop")
        .inner_size(560.0, 680.0)
        .decorations(false)
        .focused(false)
        .skip_taskbar(true)
        .on_navigation(move |url| {
            if url.path() == "/__hsg_solved" {
                if let Some((_, value)) = url.query_pairs().find(|(k, _)| k == "u") {
                    let _ = nav_tx.send(value.to_string());
                }
                return false; // never actually navigate away from the page
            }
            true
        })
        .initialization_script(WATCHER_JS);

    // macOS honours an off-screen position, so the window can stay "visible"
    // (WebKit throttles hidden ones). Windows and Linux clamp windows back on
    // screen, so there it has to be created hidden instead.
    #[cfg(target_os = "macos")]
    {
        builder = builder.position(-4000.0, -4000.0).visible(true);
    }
    #[cfg(not(target_os = "macos"))]
    {
        builder = builder.visible(false);
    }

    let window = builder.build().map_err(|e| format!("window: {e}"))?;

    // Reveal the window only if the silent pass hasn't solved it in time — the
    // same 6 s grace period the Electron build used.
    let solved = Arc::new(AtomicBool::new(false));
    {
        let show_win = window.clone();
        let solved = solved.clone();
        let handle = app.clone();
        let label = label.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(REVEAL_AFTER).await;
            if solved.load(Ordering::SeqCst) || handle.get_webview_window(&label).is_none() {
                return;
            }
            // bring it on screen and centred so the user can tap the checkbox
            let _ = show_win.center();
            let _ = show_win.show();
            let _ = show_win.set_focus();
            crate::diag::log(&handle, "turnstile: captcha needs interaction, window revealed");
        });
    }

    let deadline = tokio::time::sleep(SOLVE_TIMEOUT);
    tokio::pin!(deadline);

    let mut found: Option<String> = None;
    loop {
        tokio::select! {
            _ = &mut deadline => break,
            msg = rx.recv() => match msg {
                Some(direct) => {
                    if direct.starts_with("http") {
                        found = Some(direct);
                    }
                    break;
                }
                None => break,
            },
        }
    }

    solved.store(true, Ordering::SeqCst);
    let _ = window.close();
    drop(tx);

    match found {
        Some(direct) => {
            let short: String = direct.chars().take(90).collect();
            crate::diag::log(app, format!("turnstile {id}: solved → {short}"));
            let links = DownloadLinks {
                qr: qr_for(&direct),
                direct: Some(direct),
                ..Default::default()
            };
            if let Ok(mut map) = CACHE.lock() {
                map.insert(id.to_string(), (links.clone(), Instant::now()));
            }
            Ok(links)
        }
        None => {
            crate::diag::log(app, format!("turnstile {id}: timed out without a link"));
            Err("download link not found after captcha".to_string())
        }
    }
}

/// hShop serves a QR variant next to the direct file.
fn qr_for(direct: &str) -> Option<String> {
    if direct.contains('?') {
        Some(format!("{direct}&qr=1"))
    } else {
        Some(format!("{direct}?qr=1"))
    }
}

fn nanos_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{nanos:x}")
}
