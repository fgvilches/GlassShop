<p align="center">
  <img src="assets/hshop-logo.svg" width="128" alt="GlassShop logo">
</p>

<h1 align="center">GlassShop</h1>

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
- **Search filters** — include or exclude whole categories and regions, filter by content type, and sort; a chip per active filter that can be removed individually
- **Title sheet** with size, product code, SHA-256, description and a direct site link
- **Download queue** — configurable concurrency, pause/resume, HTTP `Range` resume, cancel, retry
- **Streaming SHA-256 verification** against the hash published by hShop
- **Live speed / ETA** and an optional bandwidth cap
- **Update detection** — compares the published version with what you downloaded, and warns when an update is being fetched without its base game (offers base-first)
- **Persistent state** — the queue survives restarts; interrupted transfers resume from the `.part` file
- **Native shell** — macOS menu bar, keyboard shortcuts (`⌘F` search, `⌘J` downloads), glass design system applied across every surface

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

## Icons

`assets/icon-source.png` is the 1024 px master and every platform icon is derived from it:

```bash
rsvg-convert -w 1024 -h 1024 -o assets/icon-source.png assets/hshop-logo.svg
npx tauri icon assets/icon-source.png -o src-tauri/icons
```

Render the SVG with `rsvg-convert` (librsvg), **not** ImageMagick's built-in SVG renderer: that one ignores the gradients and hands back a black square, which then ends up baked into every `.icns` and `.ico`.

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
```

## In-app updates

The app updates itself from GitHub Releases, on all three platforms.

1. On launch (and from **Ajustes → Actualizaciones**) it fetches
   [`latest.json`](https://github.com/fgvilches/GlassShop/releases/latest/download/latest.json),
   the manifest `tauri-action` publishes with every release.
2. If the release version is newer, it downloads the artifact for the running
   platform and arch, verifies its **minisign signature** against the public key
   baked into the binary, and swaps the bundle in place.
3. It restarts into the new version.

The signature is what makes this safe: unsigned or tampered artifacts are
rejected before anything is written. The private key lives in the
`TAURI_SIGNING_PRIVATE_KEY` / `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` repository
secrets — **if it is lost, published installs can no longer be updated** (the
public key is compiled into them).

| Platform | Artifact used by the updater |
|---|---|
| macOS | `GlassShop.app.tar.gz` (universal) |
| Windows | `-setup.exe` (NSIS) |
| Linux | `.AppImage` |

A `.deb` install cannot replace itself; in that case download the new `.deb`.
`GLASSSHOP_UPDATE_ENDPOINT` overrides the manifest URL (useful to test against
a local `latest.json`).

## How update detection works

hShop publishes a game's update as its own entry, and the two are linked by product code: the base game is `CTR-P-AXCE`, its update is `CTR-U-AXCE` (same prefix, same four-character game code, type letter swapped). GlassShop derives that code, searches hShop for it, and reads the version published on the update page.

The list of what you own lives in `library.json` next to the app data, keyed by file path and written when a download completes. **Clearing the download queue does not forget a game**, and because every entry is re-checked against the filesystem, deleting a file drops it from the library again. Files sitting in the folder that were never indexed are reported separately.

That same library answers the "update without its base" question: when you enqueue an update, the base game is looked up **on disk**, not in the download list.

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
