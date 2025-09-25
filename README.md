# EasyCLI (Tauri GUI for CLIProxyAPI)

[中文文档 | Chinese Version](README_CN.md)

EasyCLI is a Tauri v2-based desktop GUI for managing and operating CLIProxyAPI in Local or Remote mode. Recent updates migrate the app from Electron to Tauri, add a system tray with hide-to-tray behavior, and introduce proxy support during the local download/update flow.

Upstream project: https://github.com/luispater/CLIProxyAPI

## Features
- Local and Remote modes with quick switching.
- Auto-detect, download, and extract the latest CLIProxyAPI release per OS/arch.
- Version tracking under `~/cliproxyapi` and automatic config bootstrap.
- Secure remote management via password (secret key).
- System tray: Open Settings and Quit; closing the window hides to tray when the local process is running.
- Settings UI:
  - Basic: debug, port (Local), proxy URL, request logs, request retry, remote management options.
  - Access Token: manage general API access tokens.
  - Authentication Files: list/upload/download/delete JSON auth files (honors `auth-dir` with `~` and relative paths).
  - Third Party API Keys: Gemini, Codex, Claude Code.
  - OpenAI Compatibility: providers with base URLs, API keys, and optional model aliases.
- Built-in local callback server for provider auth flows (Gemini, Claude, Codex) with automatic redirection for Local/Remote modes.

## Architecture
- Frontend: static HTML/CSS + vanilla JS in `login.html`, `settings.html`, and `js/*`.
- Backend: Rust (Tauri v2) in `src-tauri/src/main.rs` exposing commands to the frontend via Tauri `invoke`.
- Packaging: Tauri bundler per `src-tauri/tauri.conf.json` (targets: dmg/app, nsis, deb).
- Data directory: `~/cliproxyapi` holds the installed CLI and configuration.

### Release Download Logic
- Checks GitHub API: `/repos/luispater/CLIProxyAPI/releases/latest`.
- Selects by OS/arch with file names:
  - macOS arm64: `CLIProxyAPI_<ver>_darwin_arm64.tar.gz`
  - macOS amd64: `CLIProxyAPI_<ver>_darwin_amd64.tar.gz`
  - Linux amd64: `CLIProxyAPI_<ver>_linux_amd64.tar.gz`
  - Windows amd64: `CLIProxyAPI_<ver>_windows_amd64.zip`
  - Windows arm64: `CLIProxyAPI_<ver>_windows_arm64.zip`
- Extracts to `~/cliproxyapi/<version>/`, writes `~/cliproxyapi/version.txt`, and ensures `config.yaml` (copied from `config.example.yaml` if present).
- Optional proxy for GitHub downloads: supports `http://`, `https://`, `socks5://` (with or without auth).

### System Tray & Window Behavior
- Tray menu: Open Settings, Quit.
- Close button hides the window to tray if the local process is running; use tray Quit to exit and stop the process.

## Requirements
- Node.js 18+ and npm 9+.
- Rust toolchain (via `rustup`) for Tauri v2.
- macOS: Xcode Command Line Tools; Windows: MSVC Build Tools; Linux: standard Tauri dependencies.

## Development
1. Install dependencies:
   ```bash
   npm install
   ```
2. Run in dev mode (watches and serves web assets, runs Tauri dev):
   ```bash
   npm run dev
   ```
   - `src-tauri/watch-web.js` mirrors `login.html`, `settings.html`, `css/`, `js/`, `images/` to `dist-web/`.
   - Tauri opens the Login window.

## Build
Use Tauri’s bundler via the npm script:
```bash
npm run build
```
Artifacts are placed under `src-tauri/target/release/bundle/` (e.g., `.dmg`/`.app` on macOS, `.nsis` on Windows, `.deb` on Linux).

## Using The App
- Local Mode
  - Optionally set a proxy in the login screen to download via HTTP/HTTPS/SOCKS5.
  - If the CLI is missing/outdated, confirm update; progress is shown.
  - When prompted, set `remote-management.secret-key` for management endpoints.
  - The app starts and monitors the local process; tray remains available.
- Remote Mode
  - Enter Base URL (e.g., `http://server:8317`) and management password.
  - The GUI reads and updates config via `/v0/management/...` endpoints.

## Data & Paths
- Root: `~/cliproxyapi`
  - `version.txt`: current version.
  - `<version>/`: extracted CLI build (`cli-proxy-api` or `cli-proxy-api.exe`).
  - `config.yaml`: active configuration; created from `config.example.yaml` if available.
- Auth files dir: from `config.yaml` key `auth-dir`; supports `~`, absolute, and relative paths (relative to `config.yaml`).

## Troubleshooting
- Cannot fetch latest release: set a proxy in Login (supports HTTP/HTTPS/SOCKS5) and retry.
- Asset not found: ensure your OS/arch matches the expected filenames listed above.
- Callback server port in use: Gemini(8085)/Claude(54545)/Codex(1455) must be free; close conflicting apps or retry.
- Hidden to tray: use the tray menu to re-open Settings or Quit.
- Config errors: ensure `version.txt` and `config.yaml` exist under `~/cliproxyapi` (Local mode bootstraps them).

## Project Layout (overview)
- `src-tauri/src/main.rs`: Tauri backend (downloads, config/auth-file ops, process + tray, callback helper).
- `src-tauri/tauri.conf.json`: Tauri configuration and bundler targets.
- `src-tauri/prepare-web.js` / `src-tauri/watch-web.js`: copy/watch static web assets to `dist-web/`.
- `login.html` + `js/login.js`: mode selection, proxy + update/install flow.
- `settings.html` + `js/settings-*.js`: settings UI (basic, tokens, API keys, OpenAI providers, auth files).
- `css/` and `images/`: UI styles and icons.

## Security Notes
- Treat the management password (secret key) as sensitive.
- Remote connection info is stored in `localStorage`; clear it on shared machines.

## License
MIT License. See `LICENSE`.
