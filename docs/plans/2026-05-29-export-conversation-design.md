# EasyGPT Conversation Export Design

## Goal

Add a top-toolbar export action that saves the currently visible AI conversation as Markdown or PDF-oriented HTML without relying on the web site's own download behavior.

## Approach

The toolbar sends an export command to Rust. Rust injects a page extraction script into the active WebView. The script reads visible conversation text from common ChatGPT, Gemini, NotebookLM, and AI Studio DOM patterns, falls back to readable `main`/`article` text, and returns structured Markdown through IPC. Rust saves the result through the same native download writer used by the download self-test.

PDF export uses a deterministic HTML document saved beside Markdown. This avoids brittle silent print dialogs while still producing a PDF-ready document that the user can open and print to PDF.

## Failure Handling

Every export path must produce a toast:

- Success: saved path is shown.
- Empty extraction: visible error says no conversation was recognized.
- IPC/script failure: visible diagnostic toast.
- Invalid filename: Rust sanitizes names before writing.

## Tests

- Shell parses Markdown/PDF export commands.
- Toolbar exposes export controls.
- Export Markdown builder escapes content and includes metadata.
- Export HTML builder wraps Markdown-derived content in printable HTML.
- Save path works through native writer and tolerates existing filenames.
- Download regressions remain covered.

## Adversarial Review Notes

- Do not call the site's own download button.
- Do not depend on one ChatGPT DOM selector.
- Do not silently ignore extraction errors.
- Do not write raw site HTML into Markdown without escaping.
- Do not assume exact filename when duplicates exist.
