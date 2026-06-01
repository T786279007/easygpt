# 跨平台 GitHub Release 实施计划

> **给 Claude/Codex：** 本计划用于维护 Windows、macOS、Linux 的自动构建与发布。

**目标：** 推送 `v*` 标签时，GitHub Actions 自动构建并发布 Windows、macOS、Linux 多平台安装/分发包。

**架构：** 三个平台分别在原生 GitHub-hosted runner 上构建，避免复杂交叉编译。各 job 上传平台 artifact，最终 release job 下载所有 artifact，生成 `SHA256SUMS.txt` 并发布 GitHub Release。

**技术栈：** GitHub Actions、Rust stable、Wry/WebView、Inno Setup、macOS `.app/.dmg`、Linux `.deb/.tar.gz`、mihomo sidecar。

---

## 任务 1：平台资源命名

**文件：**

- 修改：`src/clash.rs`
- 修改：`.gitignore`

**内容：**

- Windows 使用 `resources/clash/mihomo.exe`。
- macOS/Linux 使用 `resources/clash/mihomo`。
- macOS `.app` 还需要从 `Contents/Resources/clash/mihomo` 查找。
- `mihomo.exe` 和 `mihomo` 都不能进入 Git。

## 任务 2：数据目录兼容安装包

**文件：**

- 修改：`src/lib.rs`
- 修改：macOS/Linux 启动脚本。

**内容：**

- 默认便携模式继续使用程序旁边的 `data`。
- 安装型 macOS/Linux 通过 `EASYGPT_DATA_DIR` 指向用户可写目录。
- macOS 使用 `~/Library/Application Support/EasyGPT/data`。
- Linux 使用 `${XDG_DATA_HOME:-~/.local/share}/EasyGPT/data`。

## 任务 3：Windows 包

**文件：**

- 修改：`scripts/package-portable.ps1`
- 修改：`scripts/package-installer.ps1`
- 修改：`installer/ChatGPTWebviewClient.iss`

**产物：**

- `EasyGPT-windows-x64-Setup-版本.exe`
- `EasyGPT-windows-x64-portable.zip`

## 任务 4：macOS 包

**文件：**

- 新增：`scripts/ensure-mihomo.sh`
- 新增：`scripts/package-macos.sh`

**产物：**

- `EasyGPT-macos-arm64.dmg`
- `EasyGPT-macos-arm64-app.tar.gz`
- `EasyGPT-macos-x64.dmg`
- `EasyGPT-macos-x64-app.tar.gz`

**说明：**

- `macos-15` runner 是 arm64。
- `macos-15-intel` runner 是 Intel。
- `.dmg` 暂不签名、不公证。用户首次打开可能需要在系统安全设置中允许。

## 任务 5：Linux 包

**文件：**

- 新增：`scripts/package-linux.sh`

**产物：**

- `EasyGPT-linux-x64.deb`
- `EasyGPT-linux-x64-portable.tar.gz`

**说明：**

- Linux 构建需要 WebKitGTK、GTK、xdo 等依赖。
- `.deb` 安装到 `/opt/easygpt`，通过 `/usr/bin/easygpt` 启动。
- 便携包保留程序目录旁边的 `data`。

## 任务 6：GitHub Actions

**文件：**

- 修改：`.github/workflows/build-windows.yml`

**内容：**

- `windows` job：构建 Inno 安装器和便携包。
- `macos` matrix job：构建 arm64 与 x64。
- `linux` job：构建 x64 `.deb` 与便携包。
- `release` job：仅在 `v*` 标签触发时发布 Release。
- 发布前生成 `SHA256SUMS.txt`。

## 验证

本地可验证：

```powershell
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --release
```

Windows 本地可额外验证：

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts/package-installer.ps1 -TargetDir target_release_check
```

GitHub 端验证：

```bash
git tag v0.1.1
git push origin v0.1.1
```

检查 Release 是否包含所有平台产物和 `SHA256SUMS.txt`。
