# Proxy Console v0.3 Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add an in-page proxy console that can list mihomo strategy groups, choose nodes, test latency, refresh subscription/config state, show logs, and persist the selected proxy so ChatGPT and proxy settings are restored on next launch.

**Architecture:** Keep mihomo as the only proxy engine. Rust owns all mihomo Controller API calls and exposes a narrow IPC surface to the injected settings panel. WebView2 remains pointed only at the app-local mixed proxy, while AppData stores WebView2 login state, app settings, cached subscription, generated config, and mihomo logs.

**Tech Stack:** Rust 2024, Wry/WebView2, reqwest blocking client, serde/serde_json/serde_yaml, mihomo Controller REST API, TOML settings in `%LOCALAPPDATA%\ChatGPTWebviewClient`.

---

### Task 1: Add mihomo Controller Client

**Files:**
- Create: `src/controller.rs`
- Modify: `src/lib.rs`
- Test: `src/controller.rs`

**Steps:**
1. Write failing tests for parsing `/proxies` JSON into groups and nodes.
2. Write failing tests for URL encoding proxy names containing Chinese, spaces, and `/`.
3. Implement `ClashController` with `proxy_state`, `test_delay`, `select_proxy`, and `format_proxy_path`.
4. Export the module from `src/lib.rs`.
5. Run `cargo test`.

### Task 2: Preserve Controller Runtime and Restore Selected Node

**Files:**
- Modify: `src/clash.rs`
- Modify: `src/lib.rs`
- Test: `src/clash.rs`

**Steps:**
1. Add controller port and secret to `ClashRuntime`.
2. Add `controller()` method returning a `ClashController`.
3. After mihomo health check succeeds, restore `settings.proxy.selected_group` and `settings.proxy.selected_proxy` if both are still valid.
4. If saved node is missing, continue startup and expose a warning through UI state instead of failing ChatGPT startup.
5. Run `cargo test`.

### Task 3: Add Runtime IPC State

**Files:**
- Modify: `src/main.rs`
- Test: covered by compile and controller/clash unit tests

**Steps:**
1. Replace the static `handle_ipc_message` with an `Arc<Mutex<AppRuntimeState>>`.
2. Add IPC command handling:
   - `getProxyState`
   - `saveSettings`
   - `listProxyGroups`
   - `testDelay`
   - `testAllDelays`
   - `selectProxy`
   - `readProxyLogs`
3. Return JSON responses to the page by evaluating a global callback function.
4. Keep settings save behavior backward-compatible.
5. Run `cargo clippy --all-targets -- -D warnings`.

### Task 4: Upgrade Right-Bottom Settings UI

**Files:**
- Modify: `src/main.rs`

**Steps:**
1. Replace the simple settings form with three sections: status, subscription, node selection.
2. Add group selector, node table, per-node delay, selected marker, and action buttons.
3. Add UI states for disabled, loading, error, timeout, and saved.
4. Add log preview area for latest mihomo log lines.
5. Keep the UI compact enough to fit inside ChatGPT without blocking the main composer.

### Task 5: Subscription Refresh and Config Regeneration

**Files:**
- Modify: `src/clash.rs`
- Modify: `src/main.rs`

**Steps:**
1. Extract subscription/config generation into reusable functions.
2. Add a refresh IPC command that downloads subscription, rewrites config, and asks mihomo to reload when possible.
3. If live reload fails, keep the old runtime and show "restart required" instead of breaking the current session.
4. Run `cargo test`.

### Task 6: Docs, Packaging, and Manual Smoke

**Files:**
- Modify: `README.md`
- Run: `scripts/package-portable.ps1`

**Steps:**
1. Document node selection, delay testing, log preview, persistence, and AppData paths.
2. Run `cargo fmt`.
3. Run `cargo test`.
4. Run `cargo clippy --all-targets -- -D warnings`.
5. Build portable package into `target_proxy_console_v03`.
6. Launch the packaged EXE and verify one process starts.
