# v0.9 性能与内存优化实施计划

> **给 Claude/Codex：** 性能改动必须可验证，不用感觉判断。

**目标：** 减少卡顿和内存占用，让用户可以关闭次要 AI 页面，并手动触发内存优化，同时保留主 ChatGPT 会话。

**架构：** ChatGPT 是受保护主 WebView。其他页面懒加载，可关闭或在内存优化时释放。Windows 下隐藏 WebView 设置为低内存级别，活跃 WebView 保持正常级别。

**技术栈：** Rust、Tao、Wry/WebView、顶部 shell JavaScript、设置面板 JavaScript。

---

## 任务 1：shell 命令与测试

**文件：**

- 修改：`src/main.rs`

**步骤：**

1. 增加 `closeSite` 和 `optimizeMemory` 命令解析测试。
2. 增加顶部栏关闭按钮和内存清理按钮渲染测试。
3. 实现 `ShellCommand::CloseSite` 和 `ShellCommand::OptimizeMemory`。
4. 运行定向测试。

## 任务 2：WebView 生命周期与内存策略

**文件：**

- 修改：`src/main.rs`

**步骤：**

1. 增加可释放站点辅助函数测试。
2. 实现关闭次要 WebView 的逻辑。
3. 实现后台页面低内存策略。
4. 切换页面时恢复活跃页面正常内存级别。
5. 手动观察切换和关闭后内存变化。

## 任务 3：设置面板内存按钮

**文件：**

- 修改：`src/main.rs`

**步骤：**

1. 设置面板增加清理内存入口。
2. 点击后触发后台页面释放和 WebView GC 请求。
3. 展示操作结果。

## 验证

```powershell
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --release
```
