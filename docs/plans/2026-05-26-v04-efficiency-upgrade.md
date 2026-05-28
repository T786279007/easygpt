# v0.4 Efficiency Upgrade Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make the ChatGPT desktop client more reliable and faster to operate by stabilizing the internal proxy, improving startup behavior, reducing runtime lock contention, and adding efficient node/status controls.

**Architecture:** Keep WebView2 pointed at the configured app-local proxy port when internal Clash mode is enabled, so mihomo restarts do not invalidate the WebView proxy. Store runtime errors in app state instead of failing process startup, expose healthier status payloads through IPC, and move interactive node testing into the injected UI with incremental updates.

**Tech Stack:** Rust 2024, Wry/WebView2, tao, reqwest blocking client, mihomo controller HTTP API, injected vanilla JavaScript UI.

---

### Task 1: Fixed Internal Proxy Ports

**Files:**
- Modify: `src/clash.rs`
- Modify: `src/lib.rs`

**Steps:**
1. Write tests proving internal proxy startup settings use `mixed_port` and `controller_port`, and reject equal ports.
2. Run the focused tests and verify they fail before implementation.
3. Replace random runtime port allocation with configured settings ports.
4. Run the focused tests and full Rust tests.

### Task 2: Non-Fatal Internal Clash Startup

**Files:**
- Modify: `src/main.rs`

**Steps:**
1. Add app runtime state fields for `runtime_error` and `last_health`.
2. Start internal Clash through a non-fatal helper that returns an error string instead of aborting the app.
3. Include runtime errors in proxy state payloads so UI can show actionable status.
4. Verify with tests or string-level assertions where direct process startup is not practical.

### Task 3: Watchdog Lock Contention Reduction

**Files:**
- Modify: `src/main.rs`

**Steps:**
1. Add a helper that snapshots the current controller without performing network work while holding the global mutex.
2. Move controller health checks outside the lock.
3. Reacquire the lock only when a restart is needed.
4. Run tests and clippy.

### Task 4: Efficient Status and Node UI

**Files:**
- Modify: `src/main.rs`

**Steps:**
1. Add tests checking the injected script includes a floating status chip, quick node area, fastest-node action, cancellation action, and connectivity check action.
2. Implement a compact right-bottom status chip beside the settings button.
3. Add quick node buttons for the latest best five measured nodes.
4. Replace all-at-once backend testing in the UI with incremental per-node testing using concurrency limits and cancellation.
5. Add a fastest-node action that tests nodes and switches to the lowest delay node.
6. Add a ChatGPT connectivity check using the selected internal node.

### Task 5: Verification and Packaging

**Files:**
- Modify: `scripts/package-portable.ps1` only if packaging fails.

**Steps:**
1. Run injected JavaScript syntax check.
2. Run `cargo fmt --check`.
3. Run `cargo test`.
4. Run `cargo clippy --all-targets -- -D warnings`.
5. Package to `target_v04_efficiency`.
6. Confirm the EXE and bundled `resources/clash/mihomo.exe` exist.
