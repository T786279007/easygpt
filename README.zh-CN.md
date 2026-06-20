<div align="center">

**🌍 Languages / 语言:** [English](./README.md) · **简体中文**

</div>

<div align="center">

# 🚀 EasyGPT

**一个窗口，打开所有 AI 网页，并自带内置代理。**

轻量级 AI 网页桌面客户端，基于 Rust + Tao + Wry + 系统 WebView 构建。
在一个桌面窗口里切换 ChatGPT、Gemini、NotebookLM、Google AI Studio，并内置 mihomo/Clash 代理，开箱即用。

[![License: MIT](https://img.shields.io/badge/license-MIT-green.svg)](./LICENSE)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey.svg)](#-系统要求)
[![Rust](https://img.shields.io/badge/Rust-edition%202024-orange.svg)](https://www.rust-lang.org/)
[![Release](https://img.shields.io/badge/release-v0.1.4-blue.svg)](https://github.com/T786279007/easygpt/releases/tag/v0.1.4)
[![Made with Wry](https://img.shields.io/badge/made%20with-Wry%20%2B%20Tao-ff69b4.svg)](https://github.com/tauri-apps/wry)

[✨ 功能特性](#-功能特性) · [📥 快速下载](#-快速下载) · [📖 使用文档](#-本地运行) · [🏗️ 架构](#-架构与项目结构) · [❓ FAQ](#-faq)

</div>

---

## 📌 项目简介

EasyGPT 把分散在浏览器标签里的多个 AI 服务，收纳进一个原生桌面窗口。

- **多 AI 切换**：顶部工具栏一键切换 ChatGPT / Gemini / NotebookLM / Google AI Studio。
- **内置代理**：自带 mihomo（Clash.Meta）内核，只让本应用走代理，**不碰系统全局代理**，不影响其他应用。
- **免登录体验**：登录态保存在本地 WebView Profile，下次启动尽量保持登录。
- **跨平台**：Windows、macOS、Linux 三端原生构建，同一套功能。
- **轻量**：基于系统 WebView（WebView2 / WKWebView / WebKitGTK），不内嵌整个 Chromium，包体小、内存省。

> 适用于：需要稳定访问多个 AI 网页、希望把代理与 AI 工作流整合到一个桌面应用里的用户。

---

## ✨ 功能特性

### 🪟 多 AI 工作区
- 顶部工具栏支持 ChatGPT、Gemini、NotebookLM、Google AI Studio 多页面切换。
- 每个页面独立保留登录态与会话。

### 🌐 内置代理（mihomo / Clash.Meta）
- 支持 **直连 / 系统代理 / 内置 mihomo** 三种模式。
- 支持多个订阅链接、主动订阅选择、节点列表、节点切换、延时测试。
- 保存代理设置后**自动重新加载 AI 页面**，立即应用新的代理入口。
- 新用户设置面板内置订阅与节点引导，按步骤完成首次配置。
- 启动页显示内置代理启动进度和失败诊断。
- 查看最近 mihomo 日志，方便排障。

### 📥 下载中心
- 记录历史下载，支持搜索文件名 / 路径 / 地址 / 状态。
- 打开文件、打开所在目录、删除单条记录、清空已完成记录。
- 下载保存目录可配置，默认 `data/Downloads`。
- `blob:` / `data:` 文件由页面脚本读取后保存到本地。
- 常见后缀的 `http(s)` 链接优先使用网页会话下载，保留 ChatGPT 等站点的 Cookie 与临时授权；失败时回退到应用原生下载。

### 📤 导出
- **导出 Markdown**：提取当前页面可见对话文本。
- **导出 PDF**：Windows 优先使用 WebView2 `PrintToPdf`；其他平台使用内置 PDF 回退。
- 导出文件走同一套下载路径规则，并写入下载中心记录。

### ⚙️ 其他
- 应用设置保存在 `data/settings.toml`（TOML 格式，可手动编辑）。
- 提供内存清理按钮和后台页面低内存策略。
- GitHub Actions 自动构建 Windows / macOS / Linux 三端发布包。

---

## 📥 快速下载

> 最新版本：**v0.1.4** · [查看完整 Release](https://github.com/T786279007/easygpt/releases/tag/v0.1.4)

| 平台 | 下载 | 说明 |
| --- | --- | --- |
| 🪟 Windows x64 | [安装包](https://github.com/T786279007/easygpt/releases/download/v0.1.4/EasyGPT-windows-x64-Setup-0.1.4.exe) · [便携版](https://github.com/T786279007/easygpt/releases/download/v0.1.4/EasyGPT-windows-x64-portable.zip) | 安装包写入 `%LOCALAPPDATA%\Programs\EasyGPT` |
| 🍎 macOS Apple Silicon | [DMG](https://github.com/T786279007/easygpt/releases/download/v0.1.4/EasyGPT-macos-arm64.dmg) · [App.tar.gz](https://github.com/T786279007/easygpt/releases/download/v0.1.4/EasyGPT-macos-arm64-app.tar.gz) | M 系列 Mac |
| 🍎 macOS Intel | [DMG](https://github.com/T786279007/easygpt/releases/download/v0.1.4/EasyGPT-macos-x64.dmg) · [App.tar.gz](https://github.com/T786279007/easygpt/releases/download/v0.1.4/EasyGPT-macos-x64-app.tar.gz) | Intel Mac |
| 🐧 Linux x64 | [.deb](https://github.com/T786279007/easygpt/releases/download/v0.1.4/EasyGPT-linux-x64.deb) · [便携包](https://github.com/T786279007/easygpt/releases/download/v0.1.4/EasyGPT-linux-x64-portable.tar.gz) | Debian/Ubuntu 系 |
| 🔐 校验 | [SHA256SUMS.txt](https://github.com/T786279007/easygpt/releases/download/v0.1.4/SHA256SUMS.txt) | 下载后建议校验完整性 |

<details>
<summary>📦 全部产物列表</summary>

- `EasyGPT-windows-x64-Setup-*.exe`
- `EasyGPT-windows-x64-portable.zip`
- `EasyGPT-macos-arm64.dmg` / `EasyGPT-macos-arm64-app.tar.gz`
- `EasyGPT-macos-x64.dmg` / `EasyGPT-macos-x64-app.tar.gz`
- `EasyGPT-linux-x64.deb`
- `EasyGPT-linux-x64-portable.tar.gz`
- `SHA256SUMS.txt`

</details>

---

## 🖥️ 系统要求

### Windows
- Windows 10 / 11。
- Microsoft Edge WebView2 Runtime（多数 Win10/11 已预装）。
- 发布包内置 `resources/clash/mihomo.exe`。

### macOS
- macOS 12（Monterey）或更新版本。
- 使用系统 WKWebView。
- 发布包内置 `resources/clash/mihomo`。

### Linux
- 需要 GTK / WebKitGTK 运行环境。
- Ubuntu / Debian 运行依赖：

```bash
sudo apt install libwebkit2gtk-4.1-0 libgtk-3-0 libxdo3
```

开发或 GitHub Actions 构建时需要：

```bash
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev libxdo-dev pkg-config
```

> Linux WebView / Wry / Tauri 同类技术栈通常依赖 WebKitGTK，Debian/Ubuntu 开发包为 `libwebkit2gtk-4.1-dev`。

---

## 🏗️ 架构与项目结构

EasyGPT 用 **Rust** 编写，UI 完全由系统原生 WebView 渲染（不内嵌 Chromium），通过 `tao` 创建窗口、`wry` 托管 WebView。

```
easygpt/
├── src/                         # Rust 源码
│   ├── main.rs                  # 应用主入口：窗口、工具栏、WebView、IPC、下载、导出
│   ├── lib.rs                   # 配置模型、设置读写、数据目录解析、代理解析
│   ├── controller.rs            # 代理策略组 / 节点状态管理
│   └── clash.rs                 # 内置 mihomo 运行时：启动、订阅、配置、日志
├── scripts/                     # 各平台打包脚本
│   ├── package-portable.ps1     #   Windows 便携包
│   ├── package-installer.ps1    #   Windows Inno Setup 安装包
│   ├── package-macos.sh         #   macOS .app / .dmg
│   ├── package-linux.sh         #   Linux .deb / 便携包
│   ├── ensure-mihomo.ps1        #   Windows 准备 mihomo
│   └── ensure-mihomo.sh         #   macOS/Linux 准备 mihomo
├── installer/
│   └── ChatGPTWebviewClient.iss # Inno Setup 安装脚本
├── resources/clash/             # mihomo 内核（打包时下载，不入 Git）
├── .github/workflows/
│   └── build-windows.yml        # 多平台 CI：构建 + 发 Release
├── docs/
│   ├── plans/                   # 设计与实施计划文档
│   └── prototypes/              # UI 原型（HTML / 截图）
├── Cargo.toml                   # Rust 依赖清单
└── THIRD_PARTY_NOTICES.txt      # 第三方组件声明（mihomo）
```

**核心技术栈：** Rust 2024 edition · `tao` (窗口) · `wry` (WebView) · `reqwest` (HTTP) · `serde`/`toml` (配置) · `webview2-com` (Windows) · mihomo sidecar (代理)。

---

## 🛠️ 本地运行与构建

> 前置：已安装 [Rust toolchain](https://www.rust-lang.org/tools/install)（stable）。
> Windows 需先运行 `scripts\ensure-mihomo.ps1` 把 mihomo 放到 `resources/clash/mihomo.exe`。
> macOS/Linux 需先运行 `bash scripts/ensure-mihomo.sh`。

### 本地调试运行

```bash
cargo run
```

### Release 构建

```bash
cargo build --release
```

Windows release 产物位于：

```text
target\release\chatgpt_webview_client.exe
```

### Windows 便携包

```powershell
.\scripts\package-portable.ps1 -TargetDir target_portable
# 输出：target_portable\portable\EasyGPT
```

复制整个目录到其他 Windows 电脑即可运行。

### Windows 安装包（Inno Setup）

```powershell
.\scripts\package-installer.ps1 -TargetDir target_installer
# 输出：target_installer\installer\EasyGPT-windows-x64-Setup-0.1.4.exe
```

默认安装目录 `%LOCALAPPDATA%\Programs\EasyGPT`。如需把当前 `data` 目录打进安装源，可加 `-IncludeCurrentData`（**不要**在公开发布包中包含个人登录态、订阅链接或 Cookie）。

### macOS 发布包

```bash
bash scripts/package-macos.sh --target-dir target_macos --arch arm64
bash scripts/package-macos.sh --target-dir target_macos --arch x64
```

产物：

```text
target_macos/artifacts/EasyGPT-macos-{arm64,x64}.dmg
target_macos/artifacts/EasyGPT-macos-{arm64,x64}-app.tar.gz
```

macOS `.app` 包通过启动脚本把数据目录设置到 `~/Library/Application Support/EasyGPT/data`。

### Linux 发布包

```bash
bash scripts/package-linux.sh --target-dir target_linux --arch x64
```

产物：

```text
target_linux/artifacts/EasyGPT-linux-x64.deb
target_linux/artifacts/EasyGPT-linux-x64-portable.tar.gz
```

`.deb` 安装后通过 `easygpt` 命令启动；便携包默认把数据保存在程序目录旁边的 `data`。

---

## 🔧 数据目录与登录态

运行数据默认保存在程序旁边：

```text
data/
  WebView2Profile/     # WebView 登录态 / Cookie / LocalStorage
  settings.toml        # 应用设置
  downloads.json       # 下载历史
  Downloads/           # 默认下载目录
  clash/               # 内置 mihomo 运行文件与生成配置
```

安装型 macOS / Linux 包会通过环境变量 `EASYGPT_DATA_DIR` 把数据目录放到用户可写位置。也可以手动设置：

```bash
export EASYGPT_DATA_DIR="$HOME/.local/share/EasyGPT/data"
```

> ⚠️ **不要提交或公开发布 `data` 目录。** 它可能包含 Cookie、LocalStorage、订阅地址、代理配置、日志和下载文件。
>
> 登录态能否跨设备完全复用取决于目标网站。ChatGPT 和 Google 服务可能因设备、IP、系统密钥、浏览器指纹变化而要求重新验证。

---

## 🌐 代理功能详解

代理模式：

| 模式 | 说明 |
| --- | --- |
| `direct` | 直连，不走任何代理。 |
| `system` | 读取系统代理设置。 |
| `internal_clash` | 启动内置 mihomo，**仅本应用**走代理，不影响其他程序。 |

内置代理运行文件位于 `data/clash/`。程序**不会开启系统全局代理**，也**不会修改其他应用的代理设置**。

设置界面支持：

- 添加多个订阅链接、选择当前订阅。
- 根据提示逐步完成首次订阅和节点配置。
- 刷新订阅和 mihomo 配置。
- 查看策略组和节点、切换节点、测试节点延时。
- 保存已选策略组和节点，保存后自动重载当前 AI 页面。
- 查看最近 mihomo 日志。

---

## 🚀 GitHub Release 流程

工作流文件：`.github/workflows/build-windows.yml`。

- 推送到 `main` 或发起 PR 时：触发**构建验证**（不发布）。
- 推送 `v*` 标签时：构建**三端产物**并发布 GitHub Release，同时生成 `SHA256SUMS.txt`。

发布新版本示例：

```bash
git tag v0.1.4
git push origin v0.1.4
```

> GitHub 官方 runner 说明：`macos-15` 是 arm64，`macos-15-intel` 是 Intel；本项目据此分别构建 macOS Apple Silicon 与 Intel 包。

---

## 🔍 诊断与调试

### Windows 开启 Chrome DevTools Protocol

```powershell
$env:CHATGPT_CLIENT_REMOTE_DEBUG_PORT = "9223"
cargo run
```

然后访问：

```text
http://127.0.0.1:9223/json/list
```

本地调试文件（如 `cdp-list.json`、`debug-*.log`）已加入 `.gitignore`，不会进入 Git。

---

## 🗂️ 仓库卫生

以下内容不会进入 Git（见 `.gitignore`）：

- `target*` 构建输出。
- `data` 运行数据。
- `resources/clash/mihomo.exe`、`resources/clash/mihomo`（打包时下载）。
- 临时订阅服务和本地调试日志。

---

## ❓ FAQ

<details>
<summary><b>EasyGPT 会修改我的系统代理吗？</b></summary>

不会。即使选择 `internal_clash` 模式，也只让 EasyGPT 自身的 WebView 走内置 mihomo，**不会**开启系统全局代理，也**不会**影响浏览器或其他应用的网络。

</details>

<details>
<summary><b>为什么换了电脑 / 重装系统后要重新登录？</b></summary>

登录态保存在本地 WebView Profile 里，本质上是 Cookie 和 LocalStorage。ChatGPT、Google 等服务出于安全考虑，会根据设备、IP、系统密钥、浏览器指纹判断是否要求重新验证，因此跨设备复用不保证成功。

</details>

<details>
<summary><b>内置的 mihomo 是什么？安全吗？</b></summary>

mihomo（原 Clash.Meta）是开源的代理内核，来自 [MetaCubeX/mihomo](https://github.com/MetaCubeX/mihomo)。EasyGPT 把它作为本地 sidecar 进程启动，只供内置 WebView 使用。重新分发前请以 mihomo 上游仓库的当前许可证、源码和声明为准。详见 [`THIRD_PARTY_NOTICES.txt`](./THIRD_PARTY_NOTICES.txt)。

</details>

<details>
<summary><b>设置文件在哪？能手动改吗？</b></summary>

设置文件是 `data/settings.toml`（TOML 格式），可以用任意文本编辑器手动编辑，改完重启应用生效。

</details>

<details>
<summary><b>便携版和安装版有什么区别？</b></summary>

- **便携版**：解压即用，数据目录默认在程序旁边的 `data/`，适合放 U 盘或多机携带。
- **安装版**：写入系统安装目录，数据目录通过 `EASYGPT_DATA_DIR` 放到用户可写位置（macOS / Linux）。

</details>

<details>
<summary><b>支持自定义代理订阅吗？</b></summary>

支持。在设置面板里可添加多个订阅链接、选择当前订阅、刷新配置、查看节点、切换节点、测试延时。

</details>

---

## 📜 许可证

本项目源码基于 **[MIT License](./LICENSE)** 开源，© 2026 T786279007。

在 MIT 许可证下，你可以自由地使用、复制、修改、合并、发布、分发、再授权甚至商业销售本项目，只需保留原始版权与许可声明。

第三方组件：

- **mihomo**（[MetaCubeX/mihomo](https://github.com/MetaCubeX/mihomo)）：随发布包内置作为本地代理 sidecar，仅用于本应用 WebView 代理。mihomo 遵循其自身的开源许可证，重新分发前请以 mihomo 上游仓库的当前许可证、源码和声明为准。详见 [`THIRD_PARTY_NOTICES.txt`](./THIRD_PARTY_NOTICES.txt)。
- 其他 Rust 依赖遵循各自的许可证（见 `Cargo.lock` / crates.io）。

---

## 🤝 贡献

欢迎通过 Issue 反馈 Bug、提出功能建议，或提交 Pull Request。

- 提交 PR 前请先在本地 `cargo build` 与 `cargo run` 验证可通过。
- 请**不要**把 `data/`、`resources/clash/mihomo*`、个人登录态提交进仓库。
- 设计与实施计划见 [`docs/plans/`](./docs/plans)。

---

<div align="center">

**如果这个项目对你有帮助，欢迎 ⭐ Star 支持一下！**

Made with Rust · Tao · Wry · ❤️

</div>
