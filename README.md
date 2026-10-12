<p align="center">
  <strong>English</strong> |
  <a href="README.zh-CN.md">简体中文</a> |
  <a href="README.ja.md">日本語</a>
</p>

<p align="center">
  <img src="src/assets/logo.jpg" width="112" alt="EasyCLIProxyAPI Logo">
</p>

<h1 align="center">EasyCLIProxyAPI</h1>

<p align="center">
  A portable desktop console for CLIProxyAPI.<br>
  Our goal is to make tokens free—as in freedom.
</p>

## Overview

EasyCLIProxyAPI is a graphical desktop management tool built on
[CLIProxyAPI](https://github.com/router-for-me/CLIProxyAPI). It brings core lifecycle management,
OAuth authorization, API provider aggregation, protocol conversion, credential management,
quota inspection, usage records, model aliases, and agent client configuration into one interface.

The application is built with Tauri, React, and Rust. It can carry a matching CLIProxyAPI core
archive, making first-time setup and offline installation easier.

## Sponsor

[![https://go.apimart.ai/gh-easycliproxyapi](./assets/apimart-en.png)](https://go.apimart.ai/gh-easycliproxyapi)

Thanks to APIMart for sponsoring this project!

APIMart is a low-cost API platform for AI image & video generation — GPT-Image-2 from $0.006/image, 160+ images per dollar. One async API covers both image and video: submit a task, get an ID, fetch results via polling or callback. Batch tens of thousands of images without timeouts, switch models without changing code. Pay-as-you-go with no monthly fee — [sign up here](https://go.apimart.ai/gh-easycliproxyapi) to get started.

## Feature Tour

### Home dashboard and local API endpoints

![Home dashboard and local API endpoints](docs/screenshots/en/1.png)

The Home page provides a quick overview of the local proxy runtime and ready-to-use API endpoints:

- Start, stop, restart, and refresh the CLIProxyAPI core.
- View installation state, runtime state, process ID, and listening port.
- Copy ready-to-use OpenAI, Claude, and Gemini-compatible API endpoints.
- Check local connectivity and the application/core version at a glance.

Core installation, version comparison, and offline installation are available from the
**Version Management** page. You can switch between official GitHub, GitCode, and GitHub
mirror proxies or add custom HTTPS mirror prefixes; application and core updates prefer the selected
channel and fall back automatically.

Application and core updates are each checked once in the background at startup and automatically
whenever you enter **Version Management**. Their respective **Check for Updates** buttons remain
available for manual checks. Re-rendering the page does not trigger another check. Changing download
sources does not trigger a check, and reopening the page preserves the saved source. Checks do not
automatically download or install updates.

### OAuth account authorization

![OAuth account authorization](docs/screenshots/en/2.png)

The OAuth page centralizes browser-based authorization for supported providers:

- Codex OAuth
- Claude OAuth
- Antigravity OAuth
- Kimi OAuth
- xAI OAuth

EasyCLIProxyAPI opens the authorization page in the browser and supports completing the callback
flow when an automatic redirect is unavailable.

### API provider aggregation

![API provider aggregation](docs/screenshots/en/3.png)

The provider workspace manages upstream API credentials and endpoints by protocol or provider:

- Codex
- OpenAI-compatible providers
- DeepSeek
- Claude
- Gemini

You can add multiple connections, search existing entries, refresh provider state, and use them
through the unified local CLIProxyAPI endpoint. Requests and responses can be converted between
supported OpenAI, Claude, Gemini, and compatible formats.

### Plugins

The **Plugins** page manages installed core plugins, their enabled state and typed configuration.
Browse the plugin store to install, update or select a release version, configure additional stores
and environment-based authentication, open pages and sign in to accounts provided by enabled plugins. Third-party
installations require source review and a typed confirmation. The page detects unsupported core
builds and shows a restart action when an installation or removal requires it.

### Usage history and token analytics

![Usage history and token analytics](docs/screenshots/en/4.png)

The Usage page helps you understand local request activity and token consumption:

- Review request totals, token counts, success rate, throughput, cache hit rate, and estimated cost.
- Filter usage by time, model, provider, source, key, and result.
- Inspect request/token trends and input, output, reasoning, and cache usage.
- Browse request details, analysis views, and price statistics.
- Collect through CPA's real-time usage subscription with a durable local inbox and automatic HTTP fallback.
- Upgrade legacy usage databases once at startup after saving a backup under `usage-records/backups`.

### Agent client configuration

![Agent client configuration](docs/screenshots/en/5.png)

The Agents page detects installed desktop and CLI clients and helps connect them to the local
proxy. Supported clients include:

- Claude Code
- Claude Desktop
- Codex
- OpenCode
- OpenClaw
- Hermes Agent
- Pi (with the CLIProxyAPI provider extension)
- ZCode
- WorkBuddy / WorkBuddy AI
- Antigravity CLI
- Kimi Code
- Grok Build
- Oh My Pi

For supported clients, the application can synchronize the available model catalog, select a
default model, back up the original configuration before applying managed settings, and restore the
previous configuration.

Antigravity CLI connects through CPA's Gemini-compatible API. Launch the CLI from CPA to supply its endpoint and API key for that process. Running `agy` directly requires setting those environment variables yourself.

Codex **Session Management → Edit Context (Experimental)** reads messages only after a session is selected.
Listing and pagination query SQLite, the session index and directory metadata without opening or prefetching rollouts.
Unindexed sessions are discovered by their standard rollout filenames; missing workspace and provider details load when the editor opens.
Metadata is collapsed by default, with message-role filters retained. Close the target session before saving; saves modify real local history and create backups.
Manually renamed files without a session ID in their filename or a database path are not discovered by scanning their contents.

## Additional Capabilities

- Manage core settings, API keys, remote management credentials, and routing strategy.
- Create client-visible model aliases and map them to provider models and reasoning levels.
- Upload, download, inspect, and manage authentication files.
- Review provider quotas and account availability.
- Keep the application available from the macOS menu bar or Windows system tray.

## Quick Start

1. Download the package for your operating system from
   [GitHub Releases](https://github.com/router-for-me/EasyCLIProxyAPI/releases/latest).
2. Extract the Windows or Linux archive, or open the macOS DMG.
3. Launch EasyCLIProxyAPI.
4. Open **Version Management** and install the bundled or latest CLIProxyAPI core.
5. Return to **Home**, start the core, then copy the required local endpoint or configure an OAuth/API provider.

## Upgrading

Every Windows release publishes both the complete ZIP and the legacy `update` ZIP. This keeps in-app updates available to older clients that have not migrated yet, while newer clients use the complete package so the bundled core can be updated too.

Current Windows, Linux, and macOS release packages support in-app automatic updates. Linux replaces the portable application files while preserving runtime data, and macOS replaces the signed application bundle. Each platform waits for the new version to confirm a successful launch and rolls back automatically if startup fails. The installation directory must be writable by the current user.

Existing Linux and macOS installations need one manual upgrade to a release that includes the cross-platform auto-update marker. In-app updates are available after that release has been launched once.

If you are running v0.2.5 or earlier, perform one manual migration: exit EasyCLIProxyAPI, download the latest complete Windows ZIP for your architecture, then copy the contents of its top-level directory over the existing installation directory. Do not delete the existing directory first; user data such as `config.toml`, `oauth`, and `cpa-core/config.yaml` will remain in place. After launching the new version, later releases can use in-app automatic updates.

## Supported Platforms

GitHub Actions builds the following release packages:

| Operating System | Architecture | Package |
| --- | --- | --- |
| Windows | amd64, aarch64 | ZIP |
| macOS | amd64, aarch64 | DMG |
| Linux | amd64, aarch64 | TAR.GZ |

## Related Project

- [CLIProxyAPI](https://github.com/router-for-me/CLIProxyAPI) — the proxy core managed by this application.
