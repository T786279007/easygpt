# v0.7 稳定性与自愈实施计划

> **给 Claude/Codex：** 先写测试，再实现修复，最后跑完整验证。

**目标：** 让 AI Web 客户端可以自动恢复常见代理故障，用户打开程序后尽量不需要理解 Clash 细节也能访问 ChatGPT。

**架构：** 保持单个 Rust/Wry 可执行文件和内置 mihomo 模式。自愈逻辑放在 Rust 运行层，设置面板只展示健康状态和修复动作。

**技术栈：** Rust 2024、Wry/WebView、Tao、reqwest、serde/toml/yaml、mihomo Controller API。

---

## 任务 1：运行端口自愈

**文件：**

- 修改：`src/clash.rs`
- 测试：`src/clash.rs`

**步骤：**

1. 增加测试：空闲配置端口应保留。
2. 增加测试：相同端口应被拒绝。
3. 增加测试：端口被占用时选择新的本地端口。
4. 实现本地端口可用性检查。
5. 内置代理启动时使用可用端口。
6. 运行 `cargo test clash::tests`。

## 任务 2：清理残留 mihomo 进程

**文件：**

- 修改：`src/clash.rs`

**步骤：**

1. 启动前检测由本应用目录启动的 mihomo。
2. 只终止本应用 sidecar，不影响 Clash Verge 或其他外部代理。
3. 清理失败只记录日志，不直接崩溃。
4. 手动模拟残留进程并验证。

## 任务 3：订阅缓存可靠性

**文件：**

- 修改：`src/clash.rs`

**步骤：**

1. 每个订阅使用独立缓存文件。
2. 下载失败时，已有缓存可继续启动。
3. 缓存损坏时尝试重新下载。
4. 运行订阅缓存相关测试。

## 验证

```powershell
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
```
