# CLIProxyAPI Electron GUI

[中文文档 | Chinese Version](README_CN.md)

An Electron-based desktop GUI for managing and operating CLIProxyAPI in either Local or Remote mode. It helps you:
- Download, install, and run the latest CLIProxyAPI locally
- Configure server settings through a friendly UI
- Manage access tokens, third-party API keys, and OpenAI-compatible providers
- Browse, upload, download, and delete authentication JSON files
- Connect to a remote CLIProxyAPI instance and manage it over HTTP

> Upstream project: https://github.com/luispater/CLIProxyAPI

## macOS Notes
On first run, you may need to run the following command in Terminal:
```bash
xattr -cr cli-proxy-api-electron.app
```

## Features
- Local and Remote modes with one-click switching
- Auto-detect, download, and extract the latest CLIProxyAPI release for macOS, Linux, and Windows
- Per-OS asset selection and version tracking under `~/cliproxyapi`
- Config file bootstrap (copy `config.example.yaml` to `config.yaml` if missing)
- Secure remote management via password (secret key)
- Process lifecycle management in Local mode (start, monitor, auto-restart on critical changes)
- Settings UI:
  - Basic: debug, port (Local), proxy URL, request logs, request retry, allow localhost unauthenticated, remote management options
  - Access Token: manage general API access tokens
  - Authentication Files: list/upload/download/delete JSON auth files (honors `auth-dir` path with `~` and relative paths)
  - Third Party API Keys: Gemini, Codex, Claude Code
  - OpenAI Compatibility: providers list with base URLs, API keys, and optional model aliases

## How It Works
- Electron main process (`main.js`) creates windows and handles privileged tasks:
  - Checks GitHub releases for CLIProxyAPI (`/repos/luispater/CLIProxyAPI/releases/latest`)
  - Downloads and extracts platform-specific assets to `~/cliproxyapi/<version>` and writes `~/cliproxyapi/version.txt`
  - Ensures `~/cliproxyapi/config.yaml` exists
  - Starts/stops/monitors the local CLIProxyAPI process with `-config` when using Local mode
  - Reads/updates `config.yaml` (YAML) and manages local auth files via IPC
- Renderer (login/settings pages) provides the UI; a `ConfigManager` abstraction unifies Local vs Remote operations.
  - Remote mode calls HTTP endpoints on your server (e.g., `GET /v0/management/config`, `PUT/DELETE /v0/management/...` with `Authorization: Bearer <secret>`)

## Requirements
- Node.js 18+ (LTS recommended)
- npm 9+
- Internet access to reach GitHub Releases (for Local mode downloads)

## Quick Start (Development)
1. Install dependencies:
   ```bash
   npm install
   ```
2. Start the app in development mode:
   ```bash
   npm start
   ```
3. The app opens the Login window:
   - Local: the app checks your local CLIProxyAPI, prompts to update if outdated/missing, and then guides you to set a remote management password (secret key).
   - Remote: enter your server Base URL (e.g., `http://server:8080`) and the management password.

## Build Packages
Electron Forge makers are already configured.
- Package for your current OS:
  ```bash
  npm run make
  ```
- Generated artifacts are placed under `out/` (e.g., Squirrel installer on Windows, zip on macOS, deb/rpm on Linux).

## Data & Paths
- Install root: `~/cliproxyapi`
  - `version.txt`: current installed CLIProxyAPI version
  - `<version>/`: extracted executable (e.g., `cli-proxy-api` or `cli-proxy-api.exe`)
  - `config.yaml`: active configuration file
- Auth files directory: set by `auth-dir` inside `config.yaml`
  - Supports `~`, absolute paths, and relative paths (relative to the directory containing `config.yaml`).

## Using The App
- Local Mode
  - Click Connect. If needed, confirm update to latest CLIProxyAPI.
  - When prompted, set the remote management password (secret key) to enable management endpoints.
  - The app starts the local server on the configured `port` and monitors the process. Certain changes (e.g., port) trigger an automatic restart.
- Remote Mode
  - Provide Base URL and the server’s management password.
  - The GUI reads current config and applies changes via `/v0/management/...` endpoints.

## Troubleshooting
- Cannot fetch latest release
  - Check network and GitHub availability; corporate proxies may need `Proxy URL` configured.
- “Executable file does not exist” after download
  - Ensure your OS/arch matches the provided assets and that the release includes the expected filenames.
- “Version file does not exist” or config errors
  - The app expects `~/cliproxyapi/version.txt` and `~/cliproxyapi/config.yaml`. Use Local mode once to bootstrap.
- Password/secret issues
  - Local mode requires setting `remote-management.secret-key`. Remote mode requires the same key when connecting.
- File operations fail in Local mode
  - Verify `auth-dir` path exists or is creatable. Paths like `~/...` and relative paths are supported.

## Project Structure (overview)
- `main.js`: Electron main process, downloads/installs CLIProxyAPI, manages processes, IPC, YAML I/O
- `login.html` + `js/login.js`: mode selection and update/install flow
- `settings.html` + `js/settings-*.js`: settings UI (basic, tokens, API keys, OpenAI providers, auth files)
- `css/`: UI styles; `images/`: icons
- `forge.config.js`: Electron Forge packaging config

## Security Notes
- The management password (secret key) is sensitive; keep it private.
- Remote mode stores connection info in `localStorage` for convenience. Clear it if using shared machines.

## License
This project is released under the MIT License. See `LICENSE` for details.
