# GlassShop

A native **Liquid Glass** download manager for [hShop](https://hshop.erista.me) — browse the 3DS catalog, queue downloads with resume support, verify integrity, and get notified when a title you already downloaded has a newer version published.

Built with **Tauri 2 + Rust**. No Electron, no bundled browser: the UI is a transparent webview over a native macOS material, and all catalog scraping, downloading and hashing run in Rust.

| Platform | Status |
|---|---|
| macOS | full support (NSVisualEffectView vibrancy, universal binary via CI) |
| Windows | supported by the codebase (Mica/acrylic fallback) — built in CI |
| Linux | supported by the codebase (transparent window + compositor blur) — built in CI |

## Features

- **Catalog browsing** — categories, subcategories and title lists scraped from the site's server-rendered HTML
- **Search** by free text, Title ID, Product Code or Unique ID
- **Title sheet** with size, product code, SHA-256, description and a direct site link
- **Download queue** — configurable concurrency, pause/resume, HTTP `Range` resume, cancel, retry
- **Streaming SHA-256 verification** against the hash published by hShop
- **Live speed / ETA** and an optional bandwidth cap
- **Update detection** — compares the published version with what you downloaded, and warns when an update is being fetched without its base game (offers base-first)
- **Persistent state** — the queue survives restarts; interrupted transfers resume from the `.part` file
- **Native shell** — macOS menu bar, keyboard shortcuts (`⌘F` search, `⌘J` downloads), glass design system from `DESIGN.md`

## Requirements

- Rust **1.90+**
- Node **20+** (only for the Tauri CLI)
- Linux only: `libwebkit2gtk-4.1-dev`, `libappindicator3-dev`, `librsvg2-dev`, `patchelf`

## Development

```bash
npm install
npm run dev        # tauri dev
npm run build      # tauri build (.app/.dmg, .msi/.exe, .deb/.AppImage)
npm test           # parser unit tests
npm run test:live  # parser tests against the live site
```

## Project structure

```
src-tauri/
  src/
    lib.rs          app setup, macOS vibrancy, native menu, command registration
    commands.rs     Tauri commands — the renderer's `window.hshop` API
    catalog.rs      HTTP client + HTML parsers for the hShop catalog
    downloads.rs    download queue: workers, resume, SHA-256, speed/ETA, persistence
    turnstile.rs    Cloudflare Turnstile verification window (never bypassed)
    updates.rs      version comparison and update detection
    diag.rs         file diagnostics (<app data>/hshop.log)
  tauri.conf.json   window, bundle and security configuration
src/renderer/
  index.html        markup for every view
  app.js            state, rendering and event wiring (no framework)
  styles.css        Liquid Glass design system
  bridge.js         maps `window.hshop` onto Tauri IPC
assets/             logo source and icon master
DESIGN.md           design tokens and glass recipe
```

## How downloads are resolved

hShop gates downloads behind Cloudflare Turnstile. GlassShop **does not bypass it**. Pressing *Download* opens a real webview with the title page off-screen; Turnstile runs its normal checks, the page then renders the *Direct Download* link, and that URL is handed back to Rust through an intercepted navigation (`/__hsg_solved?u=…`). If the silent challenge fails, the window is brought on screen after a few seconds so the widget can be tapped. Resolved links are cached per title for 30 minutes.

## macOS First run

Builds are **not code-signed with a Developer ID and not notarized** — there is no certificate in the build. The binary only carries the linker's ad-hoc signature, which is why macOS shows the "unidentified developer" warning when the app arrives from a download.

Until a Developer ID is set up, executing this code on terminal.app:

```bash
xattr -dr com.apple.quarantine /Applications/GlassShop.app
```


## License

MIT — see [LICENSE](LICENSE). GlassShop is an unofficial client and is not affiliated with hShop or Nintendo.
