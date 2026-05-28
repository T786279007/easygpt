# v0.9 Performance And Memory Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Reduce perceived stutter and memory usage by letting users close secondary AI pages and trigger explicit memory optimization without losing the primary ChatGPT session.

**Architecture:** Keep ChatGPT as the protected primary WebView. Secondary pages remain lazy-created, but can be dropped on close or during memory optimization. Hidden remaining WebViews are marked low-memory on Windows WebView2; active WebView remains normal.

**Tech Stack:** Rust, Tao, Wry/WebView2, vanilla top-shell JavaScript, existing injected settings panel JavaScript.

---

### Task 1: Shell Commands And Tests

**Files:**
- Modify: `src/main.rs`

**Steps:**
1. Add failing tests for `closeSite` and `optimizeMemory` shell command parsing.
2. Add failing tests that the top shell renders close buttons for non-ChatGPT tabs and a memory optimization toolbar action.
3. Implement `ShellCommand::CloseSite` and `ShellCommand::OptimizeMemory`.
4. Verify targeted tests pass.

### Task 2: WebView Lifecycle And Memory Policy

**Files:**
- Modify: `src/main.rs`

**Steps:**
1. Add tests for pure helper `releasable_sites_for_memory()`.
2. Implement helpers to close a secondary content WebView, optimize background pages, sync top-shell tab state, and set WebView2 memory target levels on Windows.
3. Wire shell close/optimize commands on the UI thread.
4. Ensure switching pages sets active WebView to normal memory and hidden WebViews to low memory.

### Task 3: Settings Panel Memory Button

**Files:**
- Modify: `src/main.rs`

**Steps:**
1. Add failing test that settings panel exposes a memory optimization action.
2. Add `清理内存` button to the settings panel.
3. Intercept `optimizeMemory` IPC on the UI thread so content pages can trigger WebView cleanup.
4. Return a compact payload showing how many background pages were released.

### Task 4: Verification

**Commands:**
- `cargo fmt --check`
- `cargo test`
- `cargo clippy --all-targets -- -D warnings`
- `powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts/package-portable.ps1 -TargetDir target_v08_top_multipage -PackageName ChatGPTWebviewClient`

**Runtime Checks:**
- Launch the packaged EXE.
- Switch through all top tabs.
- Click close buttons for secondary tabs.
- Click memory optimization.
- Confirm the app stays alive, ChatGPT remains active, proxy still reaches `chatgpt.com`, and no bundled `mihomo.exe` remains after close.
