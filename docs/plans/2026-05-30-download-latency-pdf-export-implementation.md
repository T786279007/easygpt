# v0.4 Download Manager, Latency, and PDF Export Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add visible page latency, a usable download manager, and rendered-page PDF export.

**Architecture:** The top shell remains the control surface. Rust owns file system operations, download history, WebView2 native download callbacks, and WebView2 PDF printing. Content WebViews only provide browser-context signals via IPC and scripts.

**Tech Stack:** Rust, Tao, Wry/WebView2, in-memory runtime state, existing unit tests.

---

### Task 1: Shell UI Contract Tests

**Files:**
- Modify: `src/main.rs`

**Steps:**
- Add tests asserting the top shell contains a latency pill, download manager button, download panel, `measureLatency`, and `openDownloadPath`.
- Run targeted tests and verify they fail before implementation.

### Task 2: Download Model Tests

**Files:**
- Modify: `src/main.rs`

**Steps:**
- Add `DownloadRecord` tests for started/completed/failed/manual save events.
- Add tests for JSON payload sent to shell.
- Run targeted tests and verify they fail before implementation.

### Task 3: Implement Download Manager

**Files:**
- Modify: `src/main.rs`

**Steps:**
- Add `DownloadRecord`, `DownloadStatus`, and app-level `DownloadStore`.
- Convert `DownloadEvent` into records.
- Add shell APIs: show list, open file/location, clear completed.
- Update native and client-side download paths to emit records.

### Task 4: Implement Latency

**Files:**
- Modify: `src/main.rs`

**Steps:**
- Add `measureLatency` shell command and `LatencyEvent`.
- Inject a content WebView script that fetches the active site URL with cache-busting and timeout.
- Update shell UI on success/failure and run every 60 seconds.

### Task 5: Implement Native PDF Export

**Files:**
- Modify: `src/main.rs`
- Possibly modify: `Cargo.toml`

**Steps:**
- Add `ExportConversation(Pdf)` branch that calls WebView2 native `PrintToPdf` on Windows.
- Save into the normal download destination with a unique filename.
- On failure, use existing text PDF fallback and record the result.

### Task 6: Verify and Package

**Files:**
- No new desktop output; use project-local `target_v04_download_latency_export`.

**Commands:**
- `cargo fmt --check`
- `cargo test`
- `cargo clippy --all-targets -- -D warnings`
- `CARGO_TARGET_DIR=target_v04_download_latency_export cargo build --release`
- `powershell.exe -NoProfile -ExecutionPolicy Bypass -File ./scripts/package-installer.ps1 -TargetDir target_v04_download_latency_export`
