<div align="center">

**🌍 Languages / 语言:** **English** · [简体中文](./README.zh-CN.md)

</div>

<div align="center">

# 🚀 EasyGPT

**One window for all your AI web apps — with a built-in proxy.**

A lightweight AI web desktop client built with Rust + Tao + Wry + the system WebView.
Switch between ChatGPT, Gemini, NotebookLM, and Google AI Studio in a single desktop window, with a built-in mihomo/Clash proxy that works out of the box.

[![License: MIT](https://img.shields.io/badge/license-MIT-green.svg)](./LICENSE)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey.svg)](#-system-requirements)
[![Rust](https://img.shields.io/badge/Rust-edition%202024-orange.svg)](https://www.rust-lang.org/)
[![Release](https://img.shields.io/badge/release-v0.1.4-blue.svg)](https://github.com/T786279007/easygpt/releases/tag/v0.1.4)
[![Made with Wry](https://img.shields.io/badge/made%20with-Wry%20%2B%20Tao-ff69b4.svg)](https://github.com/tauri-apps/wry)

[✨ Features](#-features) · [📥 Download](#-quick-download) · [📖 Documentation](#-local-run--build) · [🏗️ Architecture](#-architecture--project-structure) · [❓ FAQ](#-faq)

</div>

---

## 📌 About

EasyGPT gathers the AI web services scattered across browser tabs into a single native desktop window.

- **Multi-AI switching** — switch between ChatGPT / Gemini / NotebookLM / Google AI Studio from the top toolbar.
- **Built-in proxy** — ships with the mihomo (Clash.Meta) core and routes **only this app** through it. It never touches the system-wide proxy and never affects other apps.
- **Stay signed in** — login state is persisted in a local WebView profile, so you stay logged in across restarts whenever possible.
- **Cross-platform** — native builds for Windows, macOS, and Linux with the same feature set.
- **Lightweight** — built on the system WebView (WebView2 / WKWebView / WebKitGTK) rather than bundling a full Chromium, so the binary is small and memory usage is low.

> **Who it's for:** anyone who needs stable access to multiple AI web apps and wants to keep proxy and AI workflows together in one desktop application.

---

## ✨ Features

### 🪟 Multi-AI Workspace
- Top toolbar lets you switch between ChatGPT, Gemini, NotebookLM, and Google AI Studio.
- Each page keeps its own login state and session.

### 🌐 Built-in Proxy (mihomo / Clash.Meta)
- Three modes: **Direct / System proxy / Built-in mihomo**.
- Multiple subscription links, active subscription selection, node lists, node switching, and latency testing.
- Saving proxy settings **automatically reloads the AI page** to apply the new proxy immediately.
- A guided onboarding panel walks new users through first-time subscription and node configuration.
- The splash screen shows built-in proxy startup progress and failure diagnostics.
- View recent mihomo logs for troubleshooting.

### 📥 Download Center
- Records download history with search by file name / path / URL / status.
- Open file, open containing folder, delete a single record, or clear all completed records.
- Configurable download directory (default: `data/Downloads`).
- `blob:` / `data:` files are read by page scripts and saved locally.
- Common `http(s)` links are downloaded via the web session first to preserve ChatGPT cookies and temp tokens; on failure it falls back to a native app download.

### 📤 Export
- **Export Markdown** — extract the visible conversation text on the current page.
- **Export PDF** — on Windows, uses WebView2 `PrintToPdf` first; other platforms use a built-in PDF fallback.
- Exported files go through the same download pipeline and appear in the download center.

### ⚙️ More
- App settings are stored in `data/settings.toml` (TOML, manually editable).
- Memory cleanup button and a low-memory strategy for background pages.
- GitHub Actions automatically builds Windows / macOS / Linux release packages.

---

## 📥 Quick Download

> Latest release: **v0.1.4** · [View full release](https://github.com/T786279007/easygpt/releases/tag/v0.1.4)

| Platform | Download | Notes |
| --- | --- | --- |
| 🪟 Windows x64 | [Installer](https://github.com/T786279007/easygpt/releases/download/v0.1.4/EasyGPT-windows-x64-Setup-0.1.4.exe) · [Portable](https://github.com/T786279007/easygpt/releases/download/v0.1.4/EasyGPT-windows-x64-portable.zip) | Installs to `%LOCALAPPDATA%\Programs\EasyGPT` |
| 🍎 macOS Apple Silicon | [DMG](https://github.com/T786279007/easygpt/releases/download/v0.1.4/EasyGPT-macos-arm64.dmg) · [App.tar.gz](https://github.com/T786279007/easygpt/releases/download/v0.1.4/EasyGPT-macos-arm64-app.tar.gz) | Apple Silicon Macs (M-series) |
| 🍎 macOS Intel | [DMG](https://github.com/T786279007/easygpt/releases/download/v0.1.4/EasyGPT-macos-x64.dmg) · [App.tar.gz](https://github.com/T786279007/easygpt/releases/download/v0.1.4/EasyGPT-macos-x64-app.tar.gz) | Intel Macs |
| 🐧 Linux x64 | [.deb](https://github.com/T786279007/easygpt/releases/download/v0.1.4/EasyGPT-linux-x64.deb) · [Portable](https://github.com/T786279007/easygpt/releases/download/v0.1.4/EasyGPT-linux-x64-portable.tar.gz) | Debian / Ubuntu based |
| 🔐 Checksum | [SHA256SUMS.txt](https://github.com/T786279007/easygpt/releases/download/v0.1.4/SHA256SUMS.txt) | Verify integrity after download |

<details>
<summary>📦 Full list of release artifacts</summary>

- `EasyGPT-windows-x64-Setup-*.exe`
- `EasyGPT-windows-x64-portable.zip`
- `EasyGPT-macos-arm64.dmg` / `EasyGPT-macos-arm64-app.tar.gz`
- `EasyGPT-macos-x64.dmg` / `EasyGPT-macos-x64-app.tar.gz`
- `EasyGPT-linux-x64.deb`
- `EasyGPT-linux-x64-portable.tar.gz`
- `SHA256SUMS.txt`

</details>

---

## 🖥️ System Requirements

### Windows
- Windows 10 / 11.
- Microsoft Edge WebView2 Runtime (preinstalled on most Win10/11).
- The release package bundles `resources/clash/mihomo.exe`.

### macOS
- macOS 12 (Monterey) or later.
- Uses the system WKWebView.
- The release package bundles `resources/clash/mihomo`.

### Linux
- Requires the GTK / WebKitGTK runtime.
- Runtime dependencies on Ubuntu / Debian:

```bash
sudo apt install libwebkit2gtk-4.1-0 libgtk-3-0 libxdo3
```

For development or GitHub Actions builds:

```bash
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev libxdo-dev pkg-config
```

> Linux WebView / Wry / Tauri stacks generally depend on WebKitGTK; on Debian/Ubuntu the dev package is `libwebkit2gtk-4.1-dev`.

---

## 🏗️ Architecture & Project Structure

EasyGPT is written in **Rust**. The UI is rendered entirely by the system's native WebView (no bundled Chromium). It uses `tao` to create the window and `wry` to host the WebView.

```
easygpt/
├── src/                         # Rust source
│   ├── main.rs                  # App entry: window, toolbar, WebView, IPC, downloads, export
│   ├── lib.rs                   # Config models, settings I/O, data dir resolution, proxy parsing
│   ├── controller.rs            # Proxy group / node state management
│   └── clash.rs                 # Built-in mihomo runtime: start, subscriptions, config, logs
├── scripts/                     # Per-platform packaging scripts
│   ├── package-portable.ps1     #   Windows portable
│   ├── package-installer.ps1    #   Windows Inno Setup installer
│   ├── package-macos.sh         #   macOS .app / .dmg
│   ├── package-linux.sh         #   Linux .deb / portable
│   ├── ensure-mihomo.ps1        #   Prepare mihomo (Windows)
│   └── ensure-mihomo.sh         #   Prepare mihomo (macOS/Linux)
├── installer/
│   └── ChatGPTWebviewClient.iss # Inno Setup installer script
├── resources/clash/             # mihomo core (downloaded at build time, not in Git)
├── .github/workflows/
│   └── build-windows.yml        # Multi-platform CI: build + publish release
├── docs/
│   ├── plans/                   # Design & implementation plans
│   └── prototypes/              # UI prototypes (HTML / screenshots)
├── Cargo.toml                   # Rust dependency manifest
└── THIRD_PARTY_NOTICES.txt      # Third-party notices (mihomo)
```

**Core stack:** Rust 2024 edition · `tao` (windows) · `wry` (WebView) · `reqwest` (HTTP) · `serde`/`toml` (config) · `webview2-com` (Windows) · mihomo sidecar (proxy).

---

## 🛠️ Local Run & Build

> Prerequisite: [Rust toolchain](https://www.rust-lang.org/tools/install) (stable) installed.
> On Windows, first run `scripts\ensure-mihomo.ps1` to place mihomo at `resources/clash/mihomo.exe`.
> On macOS/Linux, first run `bash scripts/ensure-mihomo.sh`.

### Debug run

```bash
cargo run
```

### Release build

```bash
cargo build --release
```

The Windows release binary is at:

```text
target\release\chatgpt_webview_client.exe
```

### Windows portable package

```powershell
.\scripts\package-portable.ps1 -TargetDir target_portable
# Output: target_portable\portable\EasyGPT
```

Copy the entire directory to another Windows PC to run it.

### Windows installer (Inno Setup)

```powershell
.\scripts\package-installer.ps1 -TargetDir target_installer
# Output: target_installer\installer\EasyGPT-windows-x64-Setup-0.1.4.exe
```

The default install directory is `%LOCALAPPDATA%\Programs\EasyGPT`. To bundle the current `data` directory into the installer source, add `-IncludeCurrentData` (**do not** include personal login state, subscription links, or cookies in public releases).

### macOS release package

```bash
bash scripts/package-macos.sh --target-dir target_macos --arch arm64
bash scripts/package-macos.sh --target-dir target_macos --arch x64
```

Artifacts:

```text
target_macos/artifacts/EasyGPT-macos-{arm64,x64}.dmg
target_macos/artifacts/EasyGPT-macos-{arm64,x64}-app.tar.gz
```

The macOS `.app` bundle sets the data directory to `~/Library/Application Support/EasyGPT/data` via a launch script.

### Linux release package

```bash
bash scripts/package-linux.sh --target-dir target_linux --arch x64
```

Artifacts:

```text
target_linux/artifacts/EasyGPT-linux-x64.deb
target_linux/artifacts/EasyGPT-linux-x64-portable.tar.gz
```

After installing the `.deb`, launch via the `easygpt` command. The portable package keeps data in a `data` folder next to the program directory by default.

---

## 🔧 Data Directory & Login State

Runtime data is stored next to the program by default:

```text
data/
  WebView2Profile/     # WebView login state / cookies / LocalStorage
  settings.toml        # App settings
  downloads.json       # Download history
  Downloads/           # Default download directory
  clash/               # Built-in mihomo runtime files and generated config
```

Installed macOS / Linux packages use the `EASYGPT_DATA_DIR` environment variable to place the data directory in a user-writable location. You can also set it manually:

```bash
export EASYGPT_DATA_DIR="$HOME/.local/share/EasyGPT/data"
```

> ⚠️ **Never commit or publish the `data` directory.** It may contain cookies, LocalStorage, subscription URLs, proxy config, logs, and downloaded files.
>
> Whether login state can be fully reused across devices depends on the target site. ChatGPT and Google services may require re-verification due to changes in device, IP, system keys, or browser fingerprint.

---

## 🌐 Proxy Details

Proxy modes:

| Mode | Description |
| --- | --- |
| `direct` | Direct connection, no proxy. |
| `system` | Read the system proxy settings. |
| `internal_clash` | Start the built-in mihomo; **only this app** is proxied, other apps are unaffected. |

The built-in proxy runtime lives in `data/clash/`. The app **never enables a system-wide proxy** and **never modifies other apps' proxy settings**.

The settings panel supports:

- Adding multiple subscription links and selecting the active subscription.
- Step-by-step first-time subscription and node configuration.
- Refreshing subscriptions and the mihomo config.
- Viewing proxy groups and nodes, switching nodes, and testing latency.
- Saving the selected group and node; the current AI page auto-reloads on save.
- Viewing recent mihomo logs.

---

## 🚀 GitHub Release Flow

Workflow file: `.github/workflows/build-windows.yml`.

- Pushing to `main` or opening a PR: triggers a **build verification** (no release).
- Pushing a `v*` tag: builds **all three platforms** and publishes a GitHub Release with a generated `SHA256SUMS.txt`.

Example of publishing a new version:

```bash
git tag v0.1.4
git push origin v0.1.4
```

> Per GitHub-hosted runner docs: `macos-15` is arm64 and `macos-15-intel` is Intel; the project builds separate Apple Silicon and Intel packages accordingly.

---

## 🔍 Diagnostics & Debugging

### Enable Chrome DevTools Protocol on Windows

```powershell
$env:CHATGPT_CLIENT_REMOTE_DEBUG_PORT = "9223"
cargo run
```

Then open:

```text
http://127.0.0.1:9223/json/list
```

Local debug files (such as `cdp-list.json`, `debug-*.log`) are already in `.gitignore` and won't be committed.

---

## 🗂️ Repository Hygiene

The following are excluded from Git (see `.gitignore`):

- `target*` build output.
- `data` runtime data.
- `resources/clash/mihomo.exe`, `resources/clash/mihomo` (downloaded at build time).
- Temporary subscription servers and local debug logs.

---

## ❓ FAQ

<details>
<summary><b>Does EasyGPT modify my system proxy?</b></summary>

No. Even in `internal_clash` mode, only EasyGPT's own WebView is routed through the built-in mihomo. It **never** enables a system-wide proxy and **never** affects your browser or other apps' networking.

</details>

<details>
<summary><b>Why do I need to sign in again after changing computers or reinstalling the OS?</b></summary>

Login state is stored in the local WebView profile, which is fundamentally cookies and LocalStorage. For security, ChatGPT, Google, and similar services decide whether to require re-verification based on device, IP, system keys, and browser fingerprint — so cross-device reuse isn't guaranteed.

</details>

<details>
<summary><b>What is the built-in mihomo? Is it safe?</b></summary>

mihomo (formerly Clash.Meta) is an open-source proxy core from [MetaCubeX/mihomo](https://github.com/MetaCubeX/mihomo). EasyGPT launches it as a local sidecar process, used only by the built-in WebView. Before redistributing, please follow the current license, source, and notices of the upstream mihomo repository. See [`THIRD_PARTY_NOTICES.txt`](./THIRD_PARTY_NOTICES.txt).

</details>

<details>
<summary><b>Where are the settings? Can I edit them manually?</b></summary>

Settings live in `data/settings.toml` (TOML format). You can edit it with any text editor; restart the app for changes to take effect.

</details>

<details>
<summary><b>What's the difference between the portable and the installer version?</b></summary>

- **Portable**: run after extraction. The data directory defaults to a `data/` folder next to the program — good for USB drives or carrying across machines.
- **Installer**: installs into the system directory; on macOS/Linux the data directory is placed in a user-writable location via `EASYGPT_DATA_DIR`.

</details>

<details>
<summary><b>Does it support custom proxy subscriptions?</b></summary>

Yes. In the settings panel you can add multiple subscription links, select the active subscription, refresh the config, view nodes, switch nodes, and test latency.

</details>

---

## 📜 License

This project is licensed under the **[MIT License](./LICENSE)**, © 2026 T786279007.

Under the MIT license, you are free to use, copy, modify, merge, publish, distribute, sublicense, and even sell this project, as long as you retain the original copyright and permission notice.

Third-party components:

- **mihomo** ([MetaCubeX/mihomo](https://github.com/MetaCubeX/mihomo)): bundled with the release as a local proxy sidecar, used only by this app's WebView. mihomo is governed by its own open-source license; before redistributing, follow the current license, source, and notices of the upstream mihomo repository. See [`THIRD_PARTY_NOTICES.txt`](./THIRD_PARTY_NOTICES.txt).
- Other Rust dependencies follow their respective licenses (see `Cargo.lock` / crates.io).

---

## 🤝 Contributing

Bug reports and feature suggestions via Issues, or Pull Requests, are welcome.

- Before opening a PR, verify locally with `cargo build` and `cargo run`.
- **Never** commit `data/`, `resources/clash/mihomo*`, or personal login state.
- See [`docs/plans/`](./docs/plans) for design and implementation plans.

---

<div align="center">

**If this project helps you, a ⭐ Star would be appreciated!**

Made with Rust · Tao · Wry · ❤️

</div>
