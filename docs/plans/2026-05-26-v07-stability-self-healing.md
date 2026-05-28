# v0.7 Stability Self-Healing Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make the AI Web Client recover from common proxy failures automatically so users can open it and access ChatGPT without manual Clash knowledge.

**Architecture:** Keep the single Rust/Wry executable and bundled mihomo model. Add self-healing in the Rust runtime layer first, then expose concise health status and repair actions through the existing injected settings panel.

**Tech Stack:** Rust 2024, Wry/WebView2, tao, reqwest blocking client, serde/toml/yaml, mihomo external controller API.

---

### Task 1: Runtime Port Self-Healing

**Files:**
- Modify: `src/clash.rs`
- Test: `src/clash.rs`

**Steps:**
1. Add tests for `configured_runtime_ports` or a new helper proving:
   - configured free ports are preserved;
   - equal configured ports are rejected or moved apart;
   - occupied ports can be replaced by available loopback ports.
2. Implement a helper that checks whether `127.0.0.1:port` can bind.
3. When internal Clash starts, use configured ports if free, otherwise choose free local ports.
4. Keep the WebView proxy pointed at the actual runtime mixed port via existing `ClashRuntime::proxy_settings`.
5. Run `cargo test clash::tests`.

### Task 2: Clean Stale Bundled Mihomo Processes

**Files:**
- Modify: `src/clash.rs`

**Steps:**
1. Before starting bundled mihomo, detect stale `mihomo.exe` processes launched from this app's portable folder.
2. Terminate only this app's bundled mihomo path, not Clash Verge or other external mihomo processes.
3. Add best-effort behavior: log cleanup failures but do not crash unless the target port remains unavailable.
4. Verify manually with a simulated leftover process if possible.

### Task 3: Subscription Cache Freshness

**Files:**
- Modify: `src/clash.rs`
- Test: `src/clash.rs`

**Steps:**
1. Keep the current behavior where auto-update plus URL prefers live subscription over cached content.
2. Add a small metadata file under `data/clash/subscription.meta.toml` with active URL hash and refreshed timestamp.
3. If active subscription changes, force download and overwrite `subscription.yaml`.
4. If download fails and existing cache belongs to the same URL, fall back to cache.
5. Run `cargo test clash::tests`.

### Task 4: Proxy State Repair API

**Files:**
- Modify: `src/main.rs`
- Modify: `src/controller.rs` if needed
- Test: `src/main.rs`, `src/controller.rs`

**Steps:**
1. Add an IPC command `repairProxy`.
2. The repair flow should restart runtime, refresh subscription, fetch proxy groups, choose a valid group/node, test ChatGPT connectivity, save selection, and return a concise result object.
3. Reuse existing controller methods where possible.
4. If no node works, return a clear error and the latest log tail.
5. Add tests for JS containing the repair action and for any pure helper logic.

### Task 5: Automatic Usable Node Selection

**Files:**
- Modify: `src/main.rs`
- Modify: `src/controller.rs`
- Test: `src/main.rs`, `src/controller.rs`

**Steps:**
1. Add a Rust helper that filters selectable groups and skips `GLOBAL` when better groups exist.
2. Prefer saved group/node when they exist and pass delay check.
3. Otherwise test nodes in small batches and choose the first node that can reach `https://chatgpt.com/cdn-cgi/trace`.
4. Save the selected group/node after a successful choice.
5. Keep the UI responsive by doing this in the IPC worker thread, not the WebView thread.

### Task 6: Startup Health and Settings UI

**Files:**
- Modify: `src/main.rs`

**Steps:**
1. Add a compact health summary to the existing centered settings panel.
2. Show subscription status, runtime status, selected group/node, ChatGPT check result, and last repair result.
3. Add one button: `一键修复`.
4. On startup/runtime ready, run a lightweight health check and update panel data when opened.
5. Do not add extra floating buttons.

### Task 7: Verification and Package

**Files:**
- Modify: `README.md`
- Package output: `target_v07_stability_self_healing/portable/ChatGPTWebviewClient`

**Steps:**
1. Run `cargo fmt --check`.
2. Run `cargo test`.
3. Run `cargo clippy --all-targets -- -D warnings`.
4. Package with `scripts/package-portable.ps1 -TargetDir target_v07_stability_self_healing -PackageName ChatGPTWebviewClient`.
5. Preserve/copy WebView2 profile data into the new portable package.
6. Start the packaged EXE and verify:
   - app stays running for at least 20 seconds;
   - bundled mihomo is running;
   - `curl.exe --proxy http://127.0.0.1:<mixed-port> https://chatgpt.com/cdn-cgi/trace` returns `h=chatgpt.com`;
   - closing the app leaves no bundled mihomo process behind.
