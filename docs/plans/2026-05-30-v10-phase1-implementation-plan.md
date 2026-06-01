# EasyGPT v0.10/v0.11 Phase 1 Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to execute this plan task-by-task.

**Goal:** Implement the first releasable slice of the v1.0 roadmap: reliable startup/proxy progress plus a persistent browser-like download center with configurable save location.

**Architecture:** Keep the existing Rust/Tao/Wry single-window architecture. Add small pure-data helpers for startup progress and downloads, then integrate them into `src/main.rs` shell IPC and WebView2 download callbacks. Keep slow proxy, filesystem, and controller operations off the UI thread.

**Tech Stack:** Rust 2024, Tao, Wry/WebView2, serde/toml/json, mihomo Controller API, vanilla shell HTML/JS.

---

## Scope

Implement in this phase:

- Startup/proxy progress state model and waiting-page UI states.
- RuntimeReady navigation fallback so the waiting page cannot remain stuck indefinitely.
- Download settings under `[downloads]`.
- Configurable download destination with `data/Downloads` default and safe fallback.
- Persistent `data/downloads.json`.
- Centered download manager UI matching the browser-style reference.
- Tests, clippy, release build, and package verification.

Do not implement in this phase:

- Long screenshot PDF.
- Automatic update.
- Full portable data import/export.
- Complex multi-thread/continued downloads.
- Tray mode.

## Task 1: Download Settings Model

**Files:**

- Modify: `src/lib.rs`
- Test: `src/lib.rs`

**Steps:**

1. Add failing tests:
   - `default_download_settings_use_portable_downloads_dir`
   - `old_settings_without_downloads_deserialize_with_defaults`
   - `app_settings_round_trip_includes_downloads`
2. Add:
   - `DownloadSettings`
   - `DownloadSaveMode`
   - defaults: fixed `data/Downloads`, empty `last_dir`, `ask_each_time = false`, `max_records = 500`
3. Add `downloads: DownloadSettings` to `AppSettings` with `#[serde(default)]`.
4. Update normalization if needed.
5. Run:

```powershell
cargo test download_settings app_settings --lib
```

Expected: all targeted tests pass.

## Task 2: Download Destination Resolver

**Files:**

- Modify: `src/main.rs`
- Test: `src/main.rs`

**Steps:**

1. Add failing tests for:
   - fixed relative directory resolves under `data/Downloads`
   - absolute fixed directory is respected
   - last directory mode uses `last_dir`
   - invalid/unwritable directory falls back to user downloads `EasyGPT`
   - duplicate filenames get counters
2. Replace `download_destination_for(path)` with a version that accepts download settings or app settings snapshot.
3. Update WebView2 native download handler, blob/data `saveDownload` handler, native self-test, and exports to use the resolver.
4. Ensure successful downloads update `last_dir` when save mode is last-dir.
5. Run targeted tests:

```powershell
cargo test download_destination unique_download_path save_download_payload --bin chatgpt_webview_client
```

Expected: targeted tests pass.

## Task 3: Persistent Download History

**Files:**

- Modify: `src/main.rs`
- Test: `src/main.rs`

**Steps:**

1. Add failing tests:
   - missing `downloads.json` loads empty history
   - corrupt JSON loads empty history and does not panic
   - record/save/load round trip preserves completed record
   - max records defaults to 500 and trims oldest completed records
   - started records from previous session become failed/cancelled on load
2. Add `download_history_path()` under `data/downloads.json`.
3. Add serializable store structs:
   - `DownloadHistoryStore`
   - `PersistedDownloadRecord`
4. Add load/save helpers with temp-file then rename for atomic save.
5. Replace `DownloadHistory::default()` startup with `load_download_history(settings.downloads.max_records)`.
6. Save after every record mutation and after clear/delete.
7. Run:

```powershell
cargo test download_history download_record --bin chatgpt_webview_client
```

Expected: targeted tests pass.

## Task 4: Download Center Shell UI

**Files:**

- Modify: `src/main.rs`
- Test: `src/main.rs`

**Steps:**

1. Add failing HTML tests:
   - shell contains centered download manager modal
   - modal includes clear completed, new download, download settings, search box
   - rows include open file, open folder, copy path, delete record, retry when possible
   - old small right-top `.download-panel` is not used as primary layout
2. Replace shell `.download-panel` with centered `.download-center`.
3. Keep toast notifications.
4. Update `renderDownloads()`:
   - render only when modal open or state changes
   - search filter by filename/path/source/url
   - use stable item id
   - show missing/failed status visibly
5. Add shell IPC parsing for:
   - `deleteDownloadRecord`
   - `retryDownload`
   - `newDownload`
   - `saveDownloadSettings`
6. Run:

```powershell
cargo test top_shell_html_exposes_download --bin chatgpt_webview_client
cargo test parse_shell_command_recognizes_download --bin chatgpt_webview_client
```

Expected: targeted tests pass.

## Task 5: Download Settings UI

**Files:**

- Modify: `src/main.rs`
- Test: `src/main.rs`

**Steps:**

1. Add failing tests that the settings panel exposes:
   - fixed save directory
   - use last directory
   - ask each time placeholder/disabled note if not implemented
   - max records
   - save and close buttons
2. Update injected settings panel HTML/JS to include a `下载` section.
3. Make `saveSettings` include `downloads` payload.
4. Ensure old settings still render without errors.
5. Run:

```powershell
cargo test settings_script --bin chatgpt_webview_client
cargo test app_settings_round_trip --lib
```

Expected: targeted tests pass.

## Task 6: Startup Progress Model

**Files:**

- Modify: `src/main.rs`
- Test: `src/main.rs`

**Steps:**

1. Add failing tests for:
   - `StartupStage` serializes expected user-facing labels
   - waiting page includes progress placeholders and action buttons
   - startup progress script updates current step and elapsed seconds
2. Add `StartupStage` enum and `StartupProgress` struct.
3. Add `UserEvent::StartupProgress`.
4. Add `startup_progress_script(progress)`.
5. Update waiting page HTML with:
   - current step
   - elapsed seconds
   - retry proxy
   - skip proxy
   - open settings
   - view logs
6. Run:

```powershell
cargo test startup_stage waiting_page startup_progress --bin chatgpt_webview_client
```

Expected: targeted tests pass.

## Task 7: RuntimeReady Fallback

**Files:**

- Modify: `src/main.rs`
- Test: `src/main.rs`

**Steps:**

1. Add failing tests:
   - RuntimeReady script navigates waiting pages for every `AiSite`
   - waiting page includes manual continue button with target URL
   - RuntimeFailed script replaces waiting page content with error actions
2. Ensure RuntimeReady fan-out runs against all existing content WebViews.
3. Ensure future content WebViews open real site when `runtime_ready == true`.
4. Add waiting page manual continue button.
5. Add timeout fallback path after 15 seconds if possible without blocking UI.
6. Run:

```powershell
cargo test runtime_ready_script waiting_page runtime_failed --bin chatgpt_webview_client
```

Expected: targeted tests pass.

## Task 8: Performance Guardrails

**Files:**

- Modify: `src/main.rs`
- Test: `src/main.rs`

**Steps:**

1. Add helper tests:
   - UI payload caps at max records
   - clear completed does not remove active downloads
   - download manager search is client-side and does not request file content
2. Add 500-record serialization/render fixture test at pure HTML/data level.
3. Add comments or helper boundaries ensuring UI thread does not perform large downloads, node tests, or JSON parsing during hot paths where practical.
4. Keep progress updates batched where possible.
5. Run:

```powershell
cargo test download_history_caps_records top_shell_html --bin chatgpt_webview_client
```

Expected: targeted tests pass.

## Task 9: Documentation

**Files:**

- Modify: `README.md`
- Modify: `docs/plans/2026-05-30-v10-product-performance-download-center-spec.md` if needed

**Steps:**

1. Document:
   - default download directory
   - download history file
   - startup progress/error page behavior
   - caveat about portable login state
2. Run no code tests for documentation-only changes unless code was touched in the same commit.

## Task 10: Final Verification

Run:

```powershell
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
$env:CARGO_TARGET_DIR='target_v10_phase1'
cargo build --release
powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts/package-installer.ps1 -TargetDir target_v10_phase1
```

Manual verification:

1. Start packaged EXE and observe at least 30 seconds.
2. Confirm startup page transitions or shows actionable error within 15 seconds.
3. Download a small file from ChatGPT or run self-test.
4. Confirm file lands in configured directory.
5. Open download center and verify record exists.
6. Close and reopen app; record still exists.
7. Open file and folder actions work.
8. Close app; bundled `mihomo.exe` is gone.

