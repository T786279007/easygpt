# easygpt

A lightweight Windows desktop client for ChatGPT, built with Rust, Wry, and Microsoft Edge WebView2.

## Features

- Opens `https://chatgpt.com` directly.
- Preserves login state between launches.
- Stores settings, WebView2 profile data, and built-in Clash runtime data in a portable `data` directory next to the EXE.
- Adds an in-page settings button for proxy configuration.
- Uses a single native Windows window with no extra frontend bundle.
- Supports system proxy, direct mode, and an app-local built-in Clash/mihomo proxy.

## Run

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

## Portable Package

```powershell
.\scripts\package-portable.ps1 -TargetDir target_v08_top_multipage
```

The portable package is created at:

```text
target_v08_top_multipage\portable\ChatGPTWebviewClient
```

Copy this whole folder to another Windows computer to run the app without
installation.

The packaging script downloads the Windows `mihomo.exe` proxy core from the
MetaCubeX/mihomo GitHub releases when `resources\clash\mihomo.exe` is missing.
The downloaded binary is intentionally ignored by Git.

## Windows Installer

The installer definition is kept at:

```text
installer\ChatGPTWebviewClient.iss
```

Install Inno Setup 6, then run:

```powershell
.\scripts\package-installer.ps1 -TargetDir target_v08_top_multipage
```

The standard installer is created at:

```text
target_v08_top_multipage\installer\ChatGPTWebviewClient-Setup-0.1.0.exe
```

The installer uses a per-user directory:

```text
%LOCALAPPDATA%\Programs\ChatGPTWebviewClient
```

This keeps the app's portable `data` directory writable without requiring
administrator permissions. Add `-IncludeCurrentData` when you intentionally want
to include the current local `data` folder in the installer source.

## Login State

Cookies, local storage, cache, and session data are saved in:

```text
.\data\WebView2Profile
```

After you log in once, reopening the EXE should keep the session unless the site
invalidates it. Copying the whole portable folder to another device also copies
the WebView2 profile, but ChatGPT, Google AI Studio, and NotebookLM may still ask
you to verify or sign in again because their cookies can be tied to device,
browser, IP, or OS-level encryption state.

## Proxy and Built-in Clash Support

The settings button in the ChatGPT window saves app settings to:

```text
.\data\settings.toml
```

On first launch after upgrading from an older build, the app copies existing
settings and `WebView2Profile` data from `%LOCALAPPDATA%\ChatGPTWebviewClient`
into `.\data` if the portable files do not already exist.

When proxy mode is `internal_clash`, the app starts the bundled mihomo core from:

```text
resources\clash\mihomo.exe
```

It downloads the configured subscription, writes a sanitized app-local config under:

```text
.\data\clash\config.yaml
```

and points only this WebView2 instance at the generated local proxy port.

Settings include proxy mode, multiple subscription URLs, active subscription
selection, subscription refresh behavior, node-selection fields, and the
preferred local ports.

If subscription update fails but a cached subscription already exists, the app
falls back to the cached file so temporary subscription outages do not block
startup.

For faster startup, the app uses the cached subscription first when one is
available. Use the proxy console's refresh button when you want to update the
subscription immediately.

The in-page proxy console can:

- show whether the built-in mihomo runtime is running;
- keep multiple subscription links, add/update/delete them, and choose the active one;
- list strategy groups and nodes from the local mihomo Controller API;
- switch the active node and save the selected group/node;
- restore the saved group/node on the next launch;
- test latency for one node or every node in the selected group;
- refresh the subscription/config when live reload is supported by mihomo;
- show the latest mihomo log lines.

To switch subscription providers, open the settings button in the lower-right
corner, choose an active subscription from the dropdown, or add a new name and
URL, then save. The selected subscription is stored in:

```text
.\data\settings.toml
```

Node delay tests run outside the WebView2 UI thread, and "test all" is limited
to small batches so a slow node does not freeze the ChatGPT page.

The current version reads the Windows user proxy setting from:

```text
HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings
```

When `ProxyEnable` is `1`, it parses `ProxyServer` and passes it to WebView2 explicitly. For example, Clash Verge's system proxy:

```text
http=127.0.0.1:7898;https=127.0.0.1:7898
```

becomes:

```text
--proxy-server=http://127.0.0.1:7898
```

You can override this by setting:

```powershell
$env:CHATGPT_CLIENT_PROXY = "http://127.0.0.1:7898"
```

A later version can add:

- node selection;
- subscription auto-refresh;
- in-app mihomo logs;
- a packaged installer.

## Diagnostics

For debugging WebView2 with Chrome DevTools Protocol:

```powershell
$env:CHATGPT_CLIENT_REMOTE_DEBUG_PORT = "9223"
cargo run
```

Then open:

```text
http://127.0.0.1:9223/json/list
````r`n
