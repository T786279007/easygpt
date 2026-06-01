# v0.8 顶部多页面切换器实施计划

> **给 Claude/Codex：** 逐项实现并在每项后运行对应测试。

**目标：** 将已确认的顶部切换器设计落到 Rust/Wry 应用中，让 ChatGPT、Gemini、NotebookLM 和 Google AI Studio 可以同时驻留并切换。

**架构：** 使用一个顶部 shell WebView 和多个内容 WebView。内容 WebView 懒加载、共享同一登录态目录、共享应用代理设置，切换时隐藏而不是销毁。

**技术栈：** Rust、Tao、Wry/WebView、mihomo、原生 JavaScript。

---

## 任务 1：站点模型与布局辅助函数

**文件：**

- 修改：`src/main.rs`

**步骤：**

1. 增加测试，确认四个站点顺序为 `chatgpt`、`gemini`、`notebooklm`、`aistudio`。
2. 增加测试，内置代理未就绪时站点初始 URL 应是等待页。
3. 增加测试，代理就绪脚本能跳转到正确站点。
4. 增加测试，内容区域为顶部栏预留固定高度。
5. 实现 `AiSite`、`TOP_BAR_HEIGHT`、`content_bounds`、`top_bar_bounds`、等待页 URL。

## 任务 2：顶部 shell WebView

**文件：**

- 修改：`src/main.rs`

**步骤：**

1. 增加顶部栏 HTML 测试。
2. 创建 shell WebView，固定在窗口顶部。
3. 通过 IPC 处理站点切换、后退、前进、刷新。
4. 调整窗口 resize 时的布局。

## 任务 3：内容 WebView 生命周期

**文件：**

- 修改：`src/main.rs`

**步骤：**

1. 默认启动 ChatGPT。
2. 其他站点首次点击时创建。
3. 切换时隐藏旧 WebView，显示新 WebView。
4. 运行测试并手动验证登录态不丢失。

## 验证

```powershell
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
```
