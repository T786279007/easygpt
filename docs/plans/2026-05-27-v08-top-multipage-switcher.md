# v0.8 Top Multipage Switcher Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build the approved top switcher into the Rust/Wry app so ChatGPT, Gemini, NotebookLM, and Google AI Studio can stay open in one EXE and switch without losing page state.

**Architecture:** Use one small child WebView as a native top shell and one child WebView per AI site as content. Content WebViews share the existing portable WebView2 profile directory and app-only proxy config, are created lazily on first activation, and are hidden instead of destroyed when switching.

**Tech Stack:** Rust, Tao, Wry/WebView2, existing mihomo/Clash runtime, vanilla injected JavaScript for the proxy settings panel.

---

### Task 1: Site Model And Layout Helpers

**Files:**
- Modify: `src/main.rs`

**Step 1: Write failing tests**

Add tests for:

- Four `AiSite` entries exist in the order `chatgpt`, `gemini`, `notebooklm`, `aistudio`.
- `site_initial_url()` returns a waiting page for internal Clash before runtime is ready.
- `runtime_ready_script_for_site()` redirects waiting pages to the correct site, not always ChatGPT.
- `content_bounds()` reserves a fixed top shell height.

**Step 2: Verify tests fail**

Run: `cargo test site_catalog_includes_all_top_tabs site_initial_url_waits_for_internal_proxy_before_ready runtime_ready_script_targets_each_site content_bounds_reserves_top_shell --bin chatgpt_webview_client`

Expected: tests fail because the helpers do not exist or still target only ChatGPT.

**Step 3: Implement helpers**

Add:

- `AiSite` enum with `key()`, `title()`, `url()`, `all()`, `default()`.
- `TOP_BAR_HEIGHT`.
- `content_bounds(width, height)` and `top_bar_bounds(width)`.
- `waiting_page_url(target_title)`.
- `site_initial_url(settings, site, runtime_ready)`.
- `runtime_ready_script_for_site(site)`.

**Step 4: Verify tests pass**

Run the same `cargo test` command and confirm all pass.

### Task 2: Top Shell WebView

**Files:**
- Modify: `src/main.rs`

**Step 1: Write failing tests**

Add tests for:

- `top_shell_html()` contains exactly four top tab buttons.
- The shell includes status pills in the toolbar and no page title/address duplicate area.
- The shell sends IPC commands `switchSite`, `navBack`, `navForward`, and `reloadActive`.

**Step 2: Verify tests fail**

Run: `cargo test top_shell --bin chatgpt_webview_client`

Expected: tests fail because the shell HTML function does not exist.

**Step 3: Implement shell HTML**

Create a compact top bar matching the approved v2 prototype:

- Left: `ChatGPT`, `Gemini`, `NotebookLM`, `Google AI Studio`.
- Right: `已登录`, `代理可用`, `页面常驻`, then back/forward/refresh icon buttons.
- Vanilla JS updates active tab locally and posts shell IPC messages.

**Step 4: Verify tests pass**

Run: `cargo test top_shell --bin chatgpt_webview_client`.

### Task 3: Multi-WebView Runtime

**Files:**
- Modify: `src/main.rs`

**Step 1: Write failing tests**

Add tests for IPC parsing:

- `parse_shell_command()` recognizes `switchSite` with a valid site.
- Invalid site keys are rejected.
- Existing settings IPC commands are not treated as shell commands.

**Step 2: Verify tests fail**

Run: `cargo test parse_shell_command --bin chatgpt_webview_client`

Expected: tests fail because command parsing is not implemented.

**Step 3: Implement runtime structure**

Replace the single full-window WebView with:

- A shell WebView built as a child at top bounds.
- A `HashMap<AiSite, WebView>` for content WebViews.
- `IpcTarget::Shell` and `IpcTarget::Site(AiSite)` so IPC responses return to the sender.
- Lazy `ensure_content_webview()` that creates and stores a site WebView on first activation.
- `switch_active_site()` that hides previous WebView, shows the target, and focuses it.

**Step 4: Wire shell commands**

Handle shell commands on the UI thread:

- `switchSite`: create/show selected site.
- `navBack`: active content WebView back.
- `navForward`: active content WebView forward.
- `reloadActive`: active content WebView reload.

**Step 5: Preserve existing settings IPC**

Forward proxy settings IPC to the background handler as before, but respond to the source content WebView instead of always the old single WebView.

**Step 6: Verify tests pass**

Run: `cargo test --bin chatgpt_webview_client`.

### Task 4: Settings Panel Cleanup

**Files:**
- Modify: `src/main.rs`

**Step 1: Write failing tests**

Update tests so the settings panel no longer contains the old site switcher.

**Step 2: Verify tests fail**

Run: `cargo test settings_script --bin chatgpt_webview_client`

Expected: old assertions fail until the switcher is removed.

**Step 3: Remove old switcher**

Remove the site switcher CSS, HTML, and `switch-site` action from `settings_button_script()`. The settings button remains the only bottom-right settings entry on the active content page.

**Step 4: Verify tests pass**

Run: `cargo test settings_script --bin chatgpt_webview_client`.

### Task 5: Resize And Runtime Ready

**Files:**
- Modify: `src/main.rs`

**Step 1: Write failing tests**

Add tests covering resize helper math and runtime ready scripts per site.

**Step 2: Verify tests fail if helpers are incomplete**

Run targeted cargo tests.

**Step 3: Implement resize handling**

On `WindowEvent::Resized`, update:

- Shell bounds to the full window width and fixed top height.
- Every content WebView bounds to start below the shell.

**Step 4: Implement runtime ready fan-out**

When the internal proxy starts:

- Mark runtime ready in the UI state.
- Run the site-specific runtime-ready script in every already-created content WebView.
- Future lazily-created WebViews open the real site URL directly.

**Step 5: Verify tests pass**

Run: `cargo test --bin chatgpt_webview_client`.

### Task 6: Build, Package, And Real EXE Verification

**Files:**
- Modify: `README.md` if usage notes need updating.

**Step 1: Format and lint**

Run:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

Expected: both pass.

**Step 2: Full tests**

Run:

```bash
cargo test
```

Expected: all tests pass.

**Step 3: Package**

Run the existing packaging script to create a new `target_v08_top_multipage` portable build.

**Step 4: Launch packaged EXE**

Start the packaged EXE and verify:

- The process remains running for at least 25 seconds.
- Bundled mihomo starts when internal Clash is enabled.
- ChatGPT trace through the app proxy returns `h=chatgpt.com`.
- No bundled mihomo process remains after closing the app.

**Step 5: Manual UI smoke**

Use the app window or screenshot tooling to verify:

- Top tabs are visible.
- Switching to Gemini/NotebookLM/Google AI Studio does not crash.
- Returning to ChatGPT keeps the app alive.
- The bottom-right settings button opens one centered settings modal.
