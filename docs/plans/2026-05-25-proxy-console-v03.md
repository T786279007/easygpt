# 代理控制台 v0.3 实施计划

> **给 Claude/Codex：** 按任务逐项执行，完成一项验证一项。

**目标：** 在页面内增加代理控制台，支持查看 mihomo 策略组、选择节点、测试延时、刷新订阅、查看日志，并保存已选节点。

**架构：** Rust 负责所有 mihomo Controller API 调用；页面设置面板只通过受控 IPC 发送命令。WebView 只使用本应用的本地 mixed 代理端口，登录态、设置、订阅缓存、生成配置和日志都保存在应用数据目录。

**技术栈：** Rust 2024、Wry/WebView、reqwest、serde/serde_json/serde_yaml、mihomo Controller REST API、TOML 设置。

---

## 任务 1：增加 mihomo Controller 客户端

**文件：**

- 新增：`src/controller.rs`
- 修改：`src/lib.rs`
- 测试：`src/controller.rs`

**步骤：**

1. 编写 `/proxies` JSON 解析测试，覆盖策略组和节点。
2. 编写代理名称 URL 编码测试，覆盖中文、空格和 `/`。
3. 实现 `ClashController::proxy_state`、`test_delay`、`select_proxy`、`format_proxy_path`。
4. 在 `src/lib.rs` 导出 controller 模块。
5. 运行 `cargo test`。

## 任务 2：保存 Controller 运行态并恢复节点

**文件：**

- 修改：`src/clash.rs`
- 修改：`src/lib.rs`
- 测试：`src/clash.rs`

**步骤：**

1. 在 `ClashRuntime` 中保存 controller 端口和密钥。
2. 增加 `controller()` 方法，返回 `ClashController`。
3. mihomo 健康检查成功后，尝试恢复 `selected_group` 和 `selected_proxy`。
4. 已保存节点不存在时，不阻塞启动，只在 UI 状态里提示。
5. 运行 `cargo test`。

## 任务 3：设置面板代理控制台

**文件：**

- 修改：`src/main.rs`

**步骤：**

1. 增加 IPC 命令：获取代理状态、选择节点、测试延时、刷新订阅、读取日志。
2. 在设置面板中展示订阅、策略组、节点、延时和运行日志。
3. 保存节点选择到 `settings.toml`。
4. 所有慢操作放到后台线程，避免卡住 WebView。
5. 运行 `cargo test` 和手动节点切换测试。

## 验证

```powershell
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
```
