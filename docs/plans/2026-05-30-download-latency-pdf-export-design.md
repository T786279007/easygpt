# v0.4 Download Manager, Latency, and PDF Export Design

## Goal

Improve the client for daily use by making connectivity visible, downloads traceable, and PDF export faithful to the rendered page.

## Scope

- Keep all generated files inside `C:\Users\admin\Desktop\新建文件夹 (8)\chatgpt_webview_client`.
- Add a top toolbar latency pill to the left of the login/proxy pills.
- Replace download-only toast behavior with a browser-like download manager panel.
- Prefer WebView2 native PDF printing for PDF export, with the existing text export as fallback.

## Design

### Latency

The shell toolbar owns the visual state. It starts with `延时 --`, then sends `measureLatency` for the active site immediately and on a fixed interval. Rust runs a small script in the active content WebView so the measurement follows the same WebView profile and proxy path as real page traffic. The shell receives `LatencyEvent` and updates the pill to `延时 123ms` or `延时失败`.

### Downloads

Rust records download events in memory for the current app session. Each entry tracks id, file name, URL, path, status, byte size when known, and timestamp. Native WebView2 download callbacks and client-side blob/data downloads both feed this list. The shell has a download button and a panel showing recent items with actions: open file, open folder, clear completed items, and close panel. Toasts remain brief notifications, but the panel is the source of truth.

### PDF Export

`导出 PDF` first calls WebView2 `PrintToPdf` on the active content WebView and writes to the Downloads folder with a unique filename. This preserves page layout far better than text extraction. If the platform API fails, Rust falls back to the existing markdown-to-PDF text path and reports the fallback in the download manager. Markdown export keeps the current text extraction path.

## Testing

- Unit tests cover shell HTML controls, IPC parsing, download list serialization, latency script plumbing, and PDF filename/path helpers.
- Full verification remains `cargo fmt --check`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, and `cargo build --release`.
