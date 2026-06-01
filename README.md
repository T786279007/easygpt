# EasyGPT

EasyGPT 是一个轻量级 AI 网页桌面客户端，基于 Rust、Tao、Wry 和系统 WebView 构建。它可以在一个桌面窗口中打开 ChatGPT、Gemini、NotebookLM 和 Google AI Studio，并支持应用内置的 mihomo/Clash 代理。

## 功能特性

- 顶部工具栏支持 ChatGPT、Gemini、NotebookLM、Google AI Studio 多页面切换。
- 登录态保存在本地 `data/WebView2Profile`，下次启动尽量免重新登录。
- 应用设置保存在 `data/settings.toml`。
- 支持直连、系统代理、手动代理、内置 mihomo 代理。
- 支持多个订阅链接、主动订阅选择、节点列表、节点切换和延时测试。
- 启动页显示内置代理启动进度和失败诊断。
- 下载中心记录历史下载，支持打开文件、打开目录、删除记录和清空已完成。
- 下载保存目录可配置，默认是 `data/Downloads`。
- 支持导出当前页面为 Markdown 或 PDF。
- 提供内存清理按钮和后台页面低内存策略。
- GitHub Actions 可自动构建 Windows、macOS、Linux 发布包。

## 系统要求

### Windows

- Windows 10/11。
- Microsoft Edge WebView2 Runtime。
- 发布包内置 `resources/clash/mihomo.exe`。

### macOS

- macOS 12 或更新版本。
- 使用系统 WKWebView。
- 发布包内置 `resources/clash/mihomo`。

### Linux

- 需要 GTK/WebKitGTK 运行环境。
- Ubuntu/Debian 可安装：

```bash
sudo apt install libwebkit2gtk-4.1-0 libgtk-3-0 libxdo3
```

开发或 GitHub Actions 构建时需要：

```bash
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev libxdo-dev pkg-config
```

Linux WebView/Wry/Tauri 同类栈通常依赖 WebKitGTK，Debian/Ubuntu 开发包为 `libwebkit2gtk-4.1-dev`。

## 本地运行

```powershell
cargo run
```

## 构建 EXE

```powershell
cargo build --release
```

Windows release 文件位于：

```text
target\release\chatgpt_webview_client.exe
```

## Windows 便携包

```powershell
.\scripts\package-portable.ps1 -TargetDir target_portable
```

输出目录：

```text
target_portable\portable\EasyGPT
```

复制整个目录到其他 Windows 电脑即可运行。

## Windows 安装包

```powershell
.\scripts\package-installer.ps1 -TargetDir target_installer
```

输出文件：

```text
target_installer\installer\EasyGPT-windows-x64-Setup-0.1.1.exe
```

安装目录默认是：

```text
%LOCALAPPDATA%\Programs\EasyGPT
```

如需把当前 `data` 目录打进安装源，可添加 `-IncludeCurrentData`。不要在公开发布包中包含个人登录态、订阅链接或 Cookie。

## macOS 发布包

```bash
bash scripts/package-macos.sh --target-dir target_macos --arch arm64
bash scripts/package-macos.sh --target-dir target_macos --arch x64
```

输出文件：

```text
target_macos/artifacts/EasyGPT-macos-arm64.dmg
target_macos/artifacts/EasyGPT-macos-arm64-app.tar.gz
target_macos/artifacts/EasyGPT-macos-x64.dmg
target_macos/artifacts/EasyGPT-macos-x64-app.tar.gz
```

macOS `.app` 包通过启动脚本把数据目录设置到：

```text
~/Library/Application Support/EasyGPT/data
```

## Linux 发布包

```bash
bash scripts/package-linux.sh --target-dir target_linux --arch x64
```

输出文件：

```text
target_linux/artifacts/EasyGPT-linux-x64.deb
target_linux/artifacts/EasyGPT-linux-x64-portable.tar.gz
```

`.deb` 安装后通过 `easygpt` 命令启动，数据目录默认是：

```text
~/.local/share/EasyGPT/data
```

便携包默认把数据保存在程序目录旁边的 `data`。

## 数据与登录态

运行数据默认保存在程序旁边：

```text
data/
  WebView2Profile/
  settings.toml
  downloads.json
  Downloads/
  clash/
```

安装型 macOS/Linux 包会通过 `EASYGPT_DATA_DIR` 把数据目录放到用户可写位置。也可以手动设置：

```bash
export EASYGPT_DATA_DIR="$HOME/.local/share/EasyGPT/data"
```

登录态能否跨设备完全复用取决于目标网站。ChatGPT 和 Google 服务可能会因为设备、IP、系统密钥、浏览器指纹变化而要求重新验证。

不要提交或公开发布 `data` 目录。它可能包含 Cookie、LocalStorage、订阅地址、代理配置、日志和下载文件。

## 代理功能

代理模式：

- `direct`：直连。
- `system`：读取系统代理。
- `manual`：使用手动代理。
- `internal_clash`：启动内置 mihomo，并且只让本应用走代理。

内置代理运行文件位于：

```text
data/clash/
```

程序不会开启系统全局代理，也不会影响其他应用。设置界面支持：

- 添加多个订阅链接。
- 选择当前订阅。
- 刷新订阅和 mihomo 配置。
- 查看策略组和节点。
- 切换节点。
- 测试节点延时。
- 保存已选策略组和节点。
- 查看最近 mihomo 日志。

## 下载中心

顶部下载按钮会打开下载中心。下载中心支持：

- 查看历史下载。
- 搜索文件名、路径、地址或状态。
- 打开文件。
- 打开所在目录。
- 删除单条记录。
- 清空已完成记录。
- 跳转下载路径设置。

下载历史保存在：

```text
data/downloads.json
```

默认下载目录：

```text
data/Downloads
```

## 导出

顶部工具栏支持：

- 导出 Markdown：提取当前页面可见对话文本。
- 导出 PDF：Windows 优先使用 WebView2 `PrintToPdf`；其他平台使用内置 PDF 回退导出。

导出文件会走同一套下载路径规则，并写入下载中心记录。

## GitHub Release

工作流文件：

```text
.github/workflows/build-windows.yml
```

推送到 `main` 或发起 PR 时会构建验证。推送 `v*` 标签时会发布 Release。

发布命令示例：

```bash
git tag v0.1.1
git push origin v0.1.1
```

Release 产物包括：

- `EasyGPT-windows-x64-Setup-*.exe`
- `EasyGPT-windows-x64-portable.zip`
- `EasyGPT-macos-arm64.dmg`
- `EasyGPT-macos-arm64-app.tar.gz`
- `EasyGPT-macos-x64.dmg`
- `EasyGPT-macos-x64-app.tar.gz`
- `EasyGPT-linux-x64.deb`
- `EasyGPT-linux-x64-portable.tar.gz`
- `SHA256SUMS.txt`

GitHub 官方 runner 说明中，`macos-15` 是 arm64，`macos-15-intel` 是 Intel；本项目据此分别构建 macOS Apple Silicon 与 Intel 包。

## 诊断

Windows WebView2 可开启 Chrome DevTools Protocol：

```powershell
$env:CHATGPT_CLIENT_REMOTE_DEBUG_PORT = "9223"
cargo run
```

然后打开：

```text
http://127.0.0.1:9223/json/list
```

本地调试文件如 `cdp-list.json` 和 `debug-*.log` 已加入忽略规则。

## 仓库卫生

以下内容不会进入 Git：

- `target*` 构建输出。
- `data` 运行数据。
- `resources/clash/mihomo.exe`。
- `resources/clash/mihomo`。
- 临时订阅服务和本地调试日志。
