# CLIProxyAPI Electron 图形界面

[English Version | 英文文档](README.md)

一个基于 Electron 的桌面图形界面，用于在本地或远程模式下管理和操作 CLIProxyAPI。它可以帮助你：
- 在本地自动下载、安装并运行最新的 CLIProxyAPI
- 通过直观的界面配置服务器参数
- 管理访问令牌、第三方 API Key，以及 OpenAI 兼容提供商
- 浏览、上传、下载和删除认证 JSON 文件
- 连接远程 CLIProxyAPI 实例并通过 HTTP 进行管理

> 上游项目地址：https://github.com/luispater/CLIProxyAPI

## macOS 用户注意事项
macOS 用户在首次运行时可能需要在命令行中执行以下操作：
```bash
xattr -cr cli-proxy-api-electron.app
```

## 功能特性
- 本地 / 远程双模式，一键切换
- 自动检测、下载并解压适配当前系统的 CLIProxyAPI 版本（macOS / Linux / Windows）
- 在 `~/cliproxyapi` 目录中维护版本信息
- 初次启动自动准备配置文件（将 `config.example.yaml` 复制为 `config.yaml`）
- 通过口令（secret key）进行远程管理鉴权
- 本地模式下的进程生命周期管理（启动、监控、关键变更时自动重启）
- 设置面板包含：
  - 基础设置：调试开关、本地端口、代理地址、请求日志、请求重试、允许本地未鉴权访问、远程管理选项
  - Access Token：管理通用访问令牌
  - 认证文件：列出 / 上传 / 下载 / 删除 JSON 认证文件（支持 `auth-dir` 路径中 `~` 与相对路径）
  - 第三方 API Key：Gemini、Codex、Claude Code
  - OpenAI 兼容：管理兼容提供商（基础 URL、API Key 与可选模型别名）

## 工作原理
- 主进程（`main.js`）负责窗口创建与特权操作：
  - 调用 GitHub Releases 接口获取最新 CLIProxyAPI 版本（`/repos/luispater/CLIProxyAPI/releases/latest`）
  - 按平台下载并解压到 `~/cliproxyapi/<version>`，同时写入 `~/cliproxyapi/version.txt`
  - 确保存在 `~/cliproxyapi/config.yaml`
  - 本地模式下以 `-config` 启动/停止/监控 CLIProxyAPI 进程
  - 通过 IPC 读写 YAML 配置与本地认证文件
- 渲染进程（登录 / 设置页面）提供 UI；`ConfigManager` 抽象层统一本地与远程操作。
  - 远程模式下通过 HTTP 调用你的服务器（如 `GET /v0/management/config`、`PUT/DELETE /v0/management/...`，并携带 `Authorization: Bearer <secret>`）

## 前置条件
- Node.js 18+（推荐 LTS）
- npm 9+
- 需要能访问 GitHub Releases（用于本地模式下载安装）

## 快速开始（开发）
1. 安装依赖：
   ```bash
   npm install
   ```
2. 启动开发模式：
   ```bash
   npm start
   ```
3. 应用会打开登录页：
   - 本地模式：检查本机 CLIProxyAPI，缺失或过期时提示更新；随后引导你设置远程管理口令（secret key）。
   - 远程模式：输入远程服务器的 Base URL（如 `http://server:8080`）和管理口令。

## 构建安装包
项目已配置 Electron Forge：
- 构建当前平台安装包：
  ```bash
  npm run make
  ```
- 构建产物位于 `out/` 目录（Windows 为 Squirrel 安装器，macOS 为 zip，Linux 为 deb/rpm）。

## 数据与目录
- 安装根目录：`~/cliproxyapi`
  - `version.txt`：记录当前已安装的 CLIProxyAPI 版本
  - `<version>/`：解压后的可执行文件（如 `cli-proxy-api` 或 `cli-proxy-api.exe`）
  - `config.yaml`：当前生效的配置文件
- 认证文件目录：由 `config.yaml` 中的 `auth-dir` 指定
  - 支持 `~`、绝对路径与相对路径（相对 `config.yaml` 所在目录）。

## 使用说明
- 本地模式
  - 点击 Connect，若需要则更新到最新 CLIProxyAPI。
  - 按提示设置远程管理口令（secret key），用于启用管理接口。
  - 应用会以配置的 `port` 启动本地服务器并监控进程；当关键配置（如端口）变更时会自动重启。
- 远程模式
  - 填写 Base URL 与管理口令。
  - 界面会读取并展示当前配置，并可通过 `/v0/management/...` 接口应用修改。

## 疑难排查
- 无法获取最新版本
  - 检查网络与 GitHub 可达性；若受限，可在“Proxy URL”中配置代理。
- 下载后提示“Executable file does not exist”
  - 确认系统与架构匹配发行资产，且发行包包含预期文件名。
- 提示“Version file does not exist”或配置错误
  - 应用期望 `~/cliproxyapi/version.txt` 与 `~/cliproxyapi/config.yaml` 存在；建议先以本地模式完成初始化。
- 口令/鉴权问题
  - 本地模式需要设置 `remote-management.secret-key`；远程模式连接需要同一口令。
- 本地文件操作失败
  - 检查 `auth-dir` 是否存在或可创建；路径如 `~/...` 与相对路径均受支持。

## 目录结构（概览）
- `main.js`：主进程，负责下载/安装 CLIProxyAPI、进程管理、IPC、YAML 读写
- `login.html` + `js/login.js`：模式选择与安装/更新流程
- `settings.html` + `js/settings-*.js`：设置界面（基础、令牌、API Key、OpenAI 兼容、认证文件）
- `css/`：样式；`images/`：图标
- `forge.config.js`：Electron Forge 打包配置

## 安全提示
- 远程管理口令（secret key）属于敏感信息，请妥善保管。
- 远程模式为便捷会将连接信息保存在 `localStorage`；在共享设备上使用后请及时清除。

## 许可协议
本项目使用 MIT 许可，详见 `LICENSE` 文件。