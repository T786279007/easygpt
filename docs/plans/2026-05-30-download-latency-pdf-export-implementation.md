# v0.4 下载中心、延时检测与 PDF 导出实施计划

> **给 Claude/Codex：** 按任务逐项实现，并在每项后运行对应测试。

**目标：** 增加可见页面延时、可用下载中心和当前页面 PDF 导出。

**架构：** 顶部 shell 是控制面。Rust 负责文件系统、下载历史、WebView 原生下载回调和 PDF 输出。内容 WebView 只负责提供当前页面上下文和脚本结果。

**技术栈：** Rust、Tao、Wry/WebView、内存运行态、serde、现有单元测试。

---

## 任务 1：shell UI 契约测试

**文件：**

- 修改：`src/main.rs`

**步骤：**

1. 增加测试，断言顶部栏包含延时状态、下载按钮、下载中心入口、`measureLatency`、`openDownloadPath`。
2. 先运行测试，确认缺功能时失败。
3. 实现最小 UI。
4. 重新运行定向测试。

## 任务 2：下载模型测试

**文件：**

- 修改：`src/main.rs`

**步骤：**

1. 增加 `DownloadRecord` 测试，覆盖开始、完成、失败、诊断事件。
2. 增加 shell 下载 payload 测试。
3. 实现 `DownloadRecord`、`DownloadStatus` 和下载历史容器。
4. 运行测试。

## 任务 3：实现下载中心

**文件：**

- 修改：`src/main.rs`

**步骤：**

1. 将 `DownloadEvent` 写入下载记录。
2. 增加 shell API：打开下载中心、打开文件、打开目录、清空已完成、删除记录。
3. 原生下载和 blob/data 下载统一写入下载历史。
4. 同步 shell 和下载管理窗口状态。

## 任务 4：实现延时检测

**文件：**

- 修改：`src/main.rs`

**步骤：**

1. 增加 `measureLatency` shell 命令和 `LatencyEvent`。
2. 向当前内容 WebView 注入 fetch 测速脚本。
3. 成功或失败都回传 shell。
4. 设置 60 秒定时检测。

## 任务 5：实现 PDF 导出

**文件：**

- 修改：`src/main.rs`
- 可能修改：`Cargo.toml`

**步骤：**

1. 增加 `ExportConversation(Pdf)` 分支。
2. Windows 调用 WebView2 `PrintToPdf`。
3. 其他平台使用内置 PDF 回退。
4. 文件写入统一下载目录。
5. 写入下载历史。

## 任务 6：验证与打包

```powershell
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
CARGO_TARGET_DIR=target_v04_download_latency_export cargo build --release
powershell.exe -NoProfile -ExecutionPolicy Bypass -File ./scripts/package-installer.ps1 -TargetDir target_v04_download_latency_export
```
