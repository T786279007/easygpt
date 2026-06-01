# EasyGPT

EasyGPT is a lightweight Windows desktop client for ChatGPT and related AI
sites, built with Rust, Tao, Wry, and Microsoft Edge WebView2.

It opens the AI pages in a native single-window shell, keeps WebView2 login
state in a portable local `data` directory, and can run an app-local
Clash/mihomo proxy without changing the system proxy for other applications.

## Features

- Top toolbar for ChatGPT, Gemini, NotebookLM, and Google AI Studio.
- Long-lived WebView2 profile under `data/WebView2Profile`.
- App-local settings under `data/settings.toml`.
- Built-in mihomo proxy mode with multiple subscription URLs.
- Node/group selection, node delay tests, and saved proxy selection.
- Startup progress and proxy failure diagnostics.
- Browser-like download manager with persistent `data/downloads.json`.
- Configurable download save path, defaulting to `data/Downloads`.
- Markdown export and WebView2 native PDF export for the active page.
- Memory cleanup control and background WebView memory-reduction hooks.
- Portable package and standard Windows installer scripts.
- GitHub Actions workflow for Windows build artifacts and tagged releases.

## Requirements

- Windows 10/11.
- Microsoft Edge WebView2 Runtime.
- Rust stable with the MSVC toolchain.
- Inno Setup 6 only when building the installer locally.

The packaging scripts download `resources/clash/mihomo.exe` automatically when
it is missing. The downloaded binary is intentionally ignored by Git.

## Run From Source

```powershell
cargo run
```

## Build EXE

```powershell
cargo build --release
```

The release executable is created at:

```text
target\release\chatgpt_webview_client.exe
```

For local source-tree runs, the app can find `resources\clash\mihomo.exe` from
the project root even when the EXE is under `target\debug` or `target\release`.

## Portable Package

```powershell
.\scripts\package-portable.ps1 -TargetDir target_portable
```

The portable package is created at:

```text
target_portable\portable\ChatGPTWebviewClient
```

Copy this whole folder to another Windows computer to run the app without a
separate installation step.

## Windows Installer

```powershell
.\scripts\package-installer.ps1 -TargetDir target_installer
```

The installer is created at:

```text
target_installer\installer\ChatGPTWebviewClient-Setup-0.1.0.exe
```

If Inno Setup 6 is not installed, the script still prepares the portable source
folder and prints the installer script path.

The installer uses a per-user directory:

```text
%LOCALAPPDATA%\Programs\ChatGPTWebviewClient
```

Use `-IncludeCurrentData` only when you intentionally want to package the
current local `data` directory into the installer source.

## Data And Login State

Runtime data is stored next to the EXE:

```text
data\
  WebView2Profile\
  settings.toml
  downloads.json
  Downloads\
  clash\
```

After signing in once, reopening the same portable folder should keep the
session unless the site invalidates it. Copying the portable folder copies the
profile too, but ChatGPT and Google services can still request verification
because their cookies may depend on device, browser, IP, or OS state.

Do not commit or publish the `data` directory. It may contain login state,
subscription URLs, proxy config, logs, downloads, and other machine-specific
files.

## Proxy

Proxy modes:

- `direct`: no proxy.
- `system`: use the Windows user proxy setting for this WebView2 instance.
- `manual`: use the proxy configured by environment/settings.
- `internal_clash`: start bundled mihomo and point only this app at it.

When `internal_clash` is enabled, EasyGPT writes sanitized mihomo runtime files
under:

```text
data\clash\
```

The built-in proxy is app-local. It does not enable the Windows global proxy and
should not affect other applications.

The settings UI supports:

- multiple subscription links;
- selecting the active subscription;
- refreshing subscription/config;
- listing proxy groups and nodes;
- switching nodes;
- testing node latency;
- saving the selected group and node for the next launch;
- viewing recent mihomo log lines.

## Downloads

The top toolbar download button opens the download manager. The manager shows
recent downloads, supports search, opens files/folders, deletes records, clears
completed records, and links to download save-path settings.

Download history is stored at:

```text
data\downloads.json
```

The default save location is:

```text
data\Downloads
```

## Export

The toolbar export menu supports:

- Markdown export through visible conversation extraction.
- PDF export through WebView2 native `PrintToPdf` on Windows, with a fallback
  record if the native path fails.

Exported files are written through the same download destination resolver used
by normal downloads.

## GitHub Actions

The workflow at `.github/workflows/build-windows.yml` runs on pushes, pull
requests, manual dispatch, and `v*` tags. It performs:

```text
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
.\scripts\package-installer.ps1 -TargetDir target_ci
```

Artifacts include a portable zip and installer. Tagged builds also publish a
GitHub Release.

## Diagnostics

For WebView2 Chrome DevTools Protocol debugging:

```powershell
$env:CHATGPT_CLIENT_REMOTE_DEBUG_PORT = "9223"
cargo run
```

Then open:

```text
http://127.0.0.1:9223/json/list
```

Local CDP lists and debug logs such as `cdp-list.json` and `debug-*.log` are
ignored by Git.

## Repository Hygiene

Tracked source files are enough to build and package the app. These files are
intentionally not tracked:

- `target*` build outputs.
- `data` runtime state.
- `resources/clash/mihomo.exe`.
- temporary subscription servers and local debug logs.
