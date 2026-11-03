// ─────────────────────────────────────────────────────────────────────────────
// Tauri bridge — exposes the exact `window.hshop` surface the Electron preload
// used to provide (src/main/preload.js), so the renderer needs no changes.
// Loaded before app.js; a no-op when running under Electron or a plain browser.
// ─────────────────────────────────────────────────────────────────────────────

(function () {
  const tauri = window.__TAURI__;
  if (!tauri || !tauri.core || window.hshop) return;

  const invoke = tauri.core.invoke;
  const listen = tauri.event && tauri.event.listen;

  // Failures come back as { error } — the same contract the IPC wrapper had.
  async function call(cmd, args) {
    try {
      return await invoke(cmd, args || {});
    } catch (err) {
      return { error: String((err && err.message) || err) };
    }
  }

  function subscribe(event, cb) {
    if (!listen) return () => {};
    let unlisten = null;
    let cancelled = false;
    listen(event, (e) => cb(e.payload)).then((un) => {
      if (cancelled) un();
      else unlisten = un;
    });
    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }

  window.hshop = {
    // catalog
    categories: () => call('catalog_categories'),
    subcategories: (categoryId) => call('catalog_subcategories', { categoryId }),
    titles: (opts) => call('catalog_titles', { opts }),
    search: (opts) => call('catalog_search', { opts }),
    titleDetail: (id) => call('title_detail', { id }),

    // downloads
    resolveDownload: (id) => call('download_resolve', { id }),
    addDownload: (info) => call('download_add', { info }),
    pause: (id) => call('download_pause', { id }),
    resume: (id) => call('download_resume', { id }),
    cancel: (id) => call('download_cancel', { id }),
    retry: (id) => call('download_retry', { id }),
    remove: (id) => call('download_remove', { id }),
    reveal: (id) => call('download_reveal', { id }),
    pauseAll: () => call('download_pause_all'),
    resumeAll: () => call('download_resume_all'),
    clearCompleted: () => call('download_clear_completed'),
    setDownloadSettings: (patch) => call('download_settings', { patch }),
    snapshot: () => call('download_snapshot'),

    // updates
    updatesSnapshot: () => call('updates_snapshot'),
    updatesCheck: () => call('updates_check'),
    updatesCheckOne: (titleId) => call('updates_check_one', { titleId }),
    updatesRecordInstalled: (info) => call('updates_record_installed', { info }),
    updatesForget: (titleId) => call('updates_forget', { titleId }),
    updatesBaseFirst: (updateInfo) => call('updates_base_first', { updateInfo }),
    updatesScan: () => call('updates_scan'),
    updatesExtras: (productCode) => call('updates_extras', { productCode }),

    // app update
    updateCurrentVersion: () => call('update_current_version'),
    updateCheck: () => call('update_check'),
    updateInstall: () => call('update_install'),
    updateRestart: () => call('update_restart'),

    // app
    openExternal: (url) => call('app_open_external', { url }),
    openDownloadsFolder: () => call('app_open_downloads'),
    pickDir: () => call('app_pick_dir'),
    platform: () => call('app_platform'),
    minimize: () => call('window_minimize'),
    toggleMaximize: () => call('window_toggle_maximize'),
    closeWindow: () => call('window_close'),

    // events
    onDownloadsChanged: (cb) => subscribe('downloads:snapshot', cb),
    onUpdatesAvailable: (cb) => subscribe('updates:available', cb),
    onUpdatesBaseFirst: (cb) => subscribe('updates:base-first', cb),
    onNavigate: (cb) => subscribe('ui:navigate', cb),
    onUpdateProgress: (cb) => subscribe('app-update:progress', cb),
    onThemeChanged: (cb) => subscribe('theme:changed', cb),
  };
})();
