# EasyCLI（CLIProxyAPI 的 Tauri 图形界面）

[English Version | 英文文档](README.md)

EasyCLI 是一个基于 Tauri v2 的桌面图形界面，用于在本地或远程模式下管理和操作 CLIProxyAPI。最近的更新将框架从 Electron 迁移到 Tauri，新增托盘（最小化到托盘）行为，并在登录流程中加入下载更新时的代理支持。

上游项目地址：https://github.com/luispater/CLIProxyAPI

## 功能特性
- 本地 / 远程双模式，快捷切换。
- 自动检测、按系统架构下载并解压最新 CLIProxyAPI。
- 在 `~/cliproxyapi` 目录维护版本与配置并自动初始化配置文件。
- 使用口令（secret key）进行远程管理鉴权。
- 系统托盘：提供“打开设置”和“退出”；当本地进程运行时关闭窗口会最小化到托盘。
- 设置界面包含：
  - 基础设置：调试开关、本地端口（本地模式）、代理 URL、请求日志、请求重试、允许 localhost 未鉴权、远程管理选项。
  - Access Token：管理通用访问令牌。
  - 认证文件：列出 / 上传 / 下载 / 删除 JSON 认证文件（支持 `auth-dir` 中的 `~` 与相对路径）。
  - 第三方 API Key：Gemini、Codex、Claude Code。
  - OpenAI 兼容：管理提供商（基础 URL、API Key 与可选模型别名）。
- 内置本地回调小型 HTTP 服务器，辅助完成 Gemini / Claude / Codex 等供应商的认证流程，并在本地/远程模式下自动重定向。

## 技术架构
- 前端：静态 HTML/CSS + 原生 JS（`login.html`、`settings.html`、`js/*`）。
- 后端：Rust（Tauri v2），位于 `src-tauri/src/main.rs`，通过 Tauri `invoke` 向前端暴露指令。
- 打包：`src-tauri/tauri.conf.json` 配置 Tauri Bundler（目标：dmg/app、nsis、deb）。
- 数据目录：`~/cliproxyapi` 存放已安装的 CLI 与配置。

### 发行版下载逻辑
- 访问 GitHub API：`/repos/luispater/CLIProxyAPI/releases/latest`。
- 根据 OS/arch 选择文件名（示例）：
  - macOS arm64：`CLIProxyAPI_<ver>_darwin_arm64.tar.gz`
  - macOS amd64：`CLIProxyAPI_<ver>_darwin_amd64.tar.gz`
  - Linux amd64：`CLIProxyAPI_<ver>_linux_amd64.tar.gz`
  - Windows amd64：`CLIProxyAPI_<ver>_windows_amd64.zip`
  - Windows arm64：`CLIProxyAPI_<ver>_windows_arm64.zip`
- 解压到 `~/cliproxyapi/<version>/`，写入 `~/cliproxyapi/version.txt`；若存在 `config.example.yaml` 则复制为 `config.yaml`。
- 登录页可配置下载代理：支持 `http://`、`https://`、`socks5://`（可携带用户名密码）。

### 托盘与窗口行为
- 托盘菜单：打开设置、退出。
- 当本地进程运行时，点击窗口关闭会最小化到托盘；通过托盘“退出”停止进程并退出应用。

## 前置条件
- Node.js 18+ 与 npm 9+。
- Rust 开发环境（通过 `rustup` 安装）以支持 Tauri v2。
- macOS：安装 Xcode Command Line Tools；Windows：MSVC Build Tools；Linux：按 Tauri 要求安装依赖。

## 开发调试
1. 安装依赖：
   ```bash
   npm install
   ```
2. 进入开发模式（监听与复制前端资源，启动 Tauri dev）：
   ```bash
   npm run dev
   ```
   - `src-tauri/watch-web.js` 将 `login.html`、`settings.html`、`css/`、`js/`、`images/` 同步至 `dist-web/`。
   - Tauri 会打开登录窗口。

## 构建与打包
使用 npm 脚本调用 Tauri 打包：
```bash
npm run build
```
构建产物位于 `src-tauri/target/release/bundle/`（macOS 为 `.dmg`/`.app`，Windows 为 `.nsis`，Linux 为 `.deb`）。

## 使用方法
- 本地模式
  - 登录页可选填代理（HTTP/HTTPS/SOCKS5）以便下载更新。
  - 若 CLI 缺失或版本过旧，确认更新；可视化显示进度。
  - 按提示设置 `remote-management.secret-key` 以启用管理接口。
  - 应用启动并监控本地进程；托盘常驻以便快捷操作。
- 远程模式
  - 输入 Base URL（如 `http://server:8317`）与管理口令。
  - 界面通过 `/v0/management/...` 读取并更新远端配置。

## 数据与目录
- 根目录：`~/cliproxyapi`
  - `version.txt`：当前版本号。
  - `<version>/`：解压后的可执行文件（`cli-proxy-api` 或 `cli-proxy-api.exe`）。
  - `config.yaml`：活动配置；若存在 `config.example.yaml` 会自动复制。
- 认证文件目录：由 `config.yaml` 键 `auth-dir` 指定；支持 `~`、绝对路径与相对路径（相对 `config.yaml` 所在目录）。

## 疑难排查
- 无法获取最新版本：在登录页配置代理（支持 HTTP/HTTPS/SOCKS5）后重试。
- 找不到匹配资产：根据上文列出的文件名确认 OS/arch 是否匹配。
- 回调端口占用：Gemini(8085) / Claude(54545) / Codex(1455) 需要空闲端口；关闭冲突应用后重试。
- 窗口“消失”：本地进程运行时关闭窗口会最小化到托盘；通过托盘菜单重新打开或退出。
- 配置错误：确保 `~/cliproxyapi` 下存在 `version.txt` 与 `config.yaml`（本地模式可自动初始化）。

## 项目结构（概览）
- `src-tauri/src/main.rs`：Tauri 后端（下载、配置/认证文件操作、进程与托盘、回调辅助）。
- `src-tauri/tauri.conf.json`：Tauri 配置与打包目标。
- `src-tauri/prepare-web.js` / `src-tauri/watch-web.js`：复制/监听静态前端资源到 `dist-web/`。
- `login.html` + `js/login.js`：模式选择、代理与安装/更新流程。
- `settings.html` + `js/settings-*.js`：设置界面（基础、令牌、API Key、OpenAI 兼容、认证文件）。
- `css/` 与 `images/`：样式与图标。

## 安全提示
- 远程管理口令（secret key）属于敏感信息，请妥善保管。
- 远程模式为便捷会将连接信息存储于 `localStorage`；共享设备上使用后请及时清除。

## 许可协议
本项目使用 MIT 许可，详见 `LICENSE` 文件。
