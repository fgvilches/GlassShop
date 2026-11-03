//! In-app updates.
//!
//! Uses Tauri's updater: it reads `latest.json` from the GitHub release, picks
//! the artifact for this platform/arch, verifies the minisign signature against
//! the public key baked into the binary, then swaps the bundle in place.
//!
//! `GLASSSHOP_UPDATE_ENDPOINT` overrides the configured endpoint — used to test
//! the flow against a local manifest without publishing a release.

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::UpdaterExt;

fn updater_for(app: &AppHandle) -> Result<tauri_plugin_updater::Updater, String> {
    let mut builder = app.updater_builder();

    if let Ok(endpoint) = std::env::var("GLASSSHOP_UPDATE_ENDPOINT") {
        if !endpoint.trim().is_empty() {
            let url = endpoint
                .parse()
                .map_err(|e| format!("endpoint inválido: {e:?}"))?;
            builder = builder
                .endpoints(vec![url])
                .map_err(|e| format!("no se pudo configurar el endpoint: {e}"))?;
        }
    }

    builder.build().map_err(|e| e.to_string())
}

/// Installed version, for the settings card.
#[tauri::command]
pub fn update_current_version(app: AppHandle) -> Value {
    json!({
        "version": app.package_info().version.to_string(),
        "platform": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
    })
}

/// Is there a newer release?
#[tauri::command]
pub async fn update_check(app: AppHandle) -> Result<Value, String> {
    let updater = updater_for(&app)?;

    match updater.check().await {
        Ok(Some(update)) => {
            crate::diag::log(
                &app,
                format!(
                    "update check: {} → {} available",
                    update.current_version, update.version
                ),
            );
            Ok(json!({
                "available": true,
                "version": update.version.to_string(),
                "currentVersion": update.current_version.to_string(),
                "notes": update.body,
                "date": update.date.map(|d| d.to_string()),
            }))
        }
        Ok(None) => {
            crate::diag::log(&app, "update check: up to date".to_string());
            Ok(json!({
                "available": false,
                "currentVersion": app.package_info().version.to_string(),
            }))
        }
        Err(e) => {
            crate::diag::log(&app, format!("update check failed: {e}"));
            Err(format!("{e}"))
        }
    }
}

/// Downloads the new version and installs it. Progress goes out as
/// `app-update:progress` events; the caller restarts afterwards.
#[tauri::command]
pub async fn update_install(app: AppHandle) -> Result<Value, String> {
    let updater = updater_for(&app)?;
    let Some(update) = updater.check().await.map_err(|e| e.to_string())? else {
        return Ok(json!({ "installed": false, "reason": "sin actualización pendiente" }));
    };

    let version = update.version.to_string();
    let handle = app.clone();

    update
        .download_and_install(
            move |chunk: usize, total: Option<u64>| {
                let _ = handle.emit(
                    "app-update:progress",
                    json!({ "chunk": chunk, "total": total }),
                );
            },
            || {},
        )
        .await
        .map_err(|e| {
            crate::diag::log(&app, format!("update install failed: {e}"));
            format!("{e}")
        })?;

    crate::diag::log(&app, format!("update {version} installed"));
    Ok(json!({ "installed": true, "version": version }))
}

/// Restarts into the freshly installed version.
#[tauri::command]
pub fn update_restart(app: AppHandle) {
    app.restart();
}

#[cfg(test)]
mod tests {
    /// The public key baked into the binary must match the key that signs the
    /// updater artifacts — otherwise every install fails verification.
    /// Run with SIG_FILE and ARTIFACT_FILE set after a real build.
    #[test]
    #[ignore]
    fn pubkey_matches_signed_artifact() {
        let sig_path = std::env::var("SIG_FILE").expect("SIG_FILE");
        let art_path = std::env::var("ARTIFACT_FILE").expect("ARTIFACT_FILE");

        let conf: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string("tauri.conf.json").unwrap()).unwrap();
        let pubkey_b64 = conf["plugins"]["updater"]["pubkey"].as_str().unwrap();

        // the pubkey in the config is the base64 of the minisign public key file
        let pem = String::from_utf8(base64_decode(pubkey_b64)).expect("pubkey is base64 text");
        let public_key = minisign_verify::PublicKey::decode(&pem).expect("valid minisign pubkey");

        let signature = minisign_verify::Signature::decode(&String::from_utf8(base64_decode(
            &std::fs::read_to_string(&sig_path).expect("signature file"),
        ))
        .expect("signature is base64 text"))
        .expect("valid signature file");
        let data = std::fs::read(&art_path).expect("artifact file");

        public_key
            .verify(&data, &signature, false)
            .expect("signature must verify against the baked pubkey");
        println!("signature ok for {} ({} bytes)", art_path, data.len());
    }

    fn base64_decode(input: &str) -> Vec<u8> {
        const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = Vec::new();
        let mut buf: u32 = 0;
        let mut bits = 0;
        for c in input.bytes() {
            if c == b'=' || c == b'\n' || c == b'\r' {
                continue;
            }
            let Some(v) = T.iter().position(|&t| t == c) else {
                continue;
            };
            buf = (buf << 6) | v as u32;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push((buf >> bits) as u8);
            }
        }
        out
    }
}
