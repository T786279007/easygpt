# EasyGPT v0.10/v0.11 第一阶段实施计划

> **给 Claude/Codex：** 这是阶段性实施计划，优先保证启动可靠和下载中心可用。

**目标：** 交付 v1.0 路线中的第一批可发布能力：可靠启动/代理进度，以及持久化下载中心和可配置保存位置。

**架构：** 保持现有 Rust/Tao/Wry 单窗口结构。新增小型纯数据模型管理启动进度和下载历史，再接入 `src/main.rs` 的 shell IPC 和 WebView 下载回调。代理、文件系统和 Controller 慢操作不得阻塞 UI 线程。

**技术栈：** Rust 2024、Tao、Wry/WebView、serde/toml/json、mihomo Controller API、原生 HTML/JS。

---

## 范围

本阶段实现：

- 启动/代理进度状态模型。
- 等待页阶段状态展示。
- `RuntimeReady` 导航兜底，避免一直卡在等待页。
- `[downloads]` 设置。
- 下载保存目录配置。
- 默认 `data/Downloads`。
- 持久化 `data/downloads.json`。
- 居中浏览器式下载中心。
- 测试、clippy、release 构建和安装包验证。

本阶段不实现：

- 长截图 PDF。
- 自动更新。
- 完整便携数据导入/导出。
- 复杂多线程断点续传下载。
- 托盘模式。

## 任务 1：下载设置模型

**文件：**

- 修改：`src/lib.rs`
- 测试：`src/lib.rs`

**步骤：**

1. 增加默认下载设置测试。
2. 增加旧配置缺少 `[downloads]` 时的兼容测试。
3. 增加设置 TOML 往返测试。
4. 添加 `DownloadSettings`、`DownloadSaveMode`。
5. 在 `AppSettings` 增加 `downloads` 字段并设置默认值。
6. 运行 `cargo test download_settings app_settings --lib`。

## 任务 2：下载目录解析

**文件：**

- 修改：`src/main.rs`
- 测试：`src/main.rs`

**步骤：**

1. 增加固定相对目录解析测试。
2. 增加固定绝对目录解析测试。
3. 增加最近目录模式测试。
4. 增加同名文件自动加序号测试。
5. 替换旧的下载路径函数。
6. 下载成功后更新 `last_dir`。

## 任务 3：下载历史持久化

**文件：**

- 修改：`src/main.rs`
- 测试：`src/main.rs`

**步骤：**

1. 增加缺少 `downloads.json` 时返回空历史测试。
2. 增加历史记录上限测试。
3. 增加启动中断的下载标记失败测试。
4. 实现 `load_download_history` 和 `save_download_history`。

## 任务 4：启动进度与错误页

**文件：**

- 修改：`src/main.rs`

**步骤：**

1. 定义启动阶段枚举。
2. 等待页显示当前步骤、已用时间和操作提示。
3. 代理启动线程分阶段发送事件。
4. 失败时显示错误页，不让窗口闪退。

## 任务 5：下载中心 UI

**文件：**

- 修改：`src/main.rs`

**步骤：**

1. 顶部工具栏增加下载按钮。
2. 打开居中下载中心窗口。
3. 支持搜索、打开文件、打开目录、删除记录、清空已完成、关闭。
4. 下载记录变化时同步窗口。

## 任务 6：最终验证

```powershell
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --release
powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts/package-installer.ps1 -TargetDir target_v10_phase1
```
