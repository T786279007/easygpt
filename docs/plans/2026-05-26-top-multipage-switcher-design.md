# Top Multipage Switcher Design

## Goal

Add a compact top switcher for ChatGPT, Gemini, NotebookLM, and Google AI Studio while keeping the app as a lightweight single-window client.

## Recommended Interaction

Use a browser-like top bar. The left side contains four pinned page buttons. Clicking a page starts it if it has not been opened yet, then switches to it. Started pages remain alive while hidden, so switching does not discard login state, page state, or temporary work.

The right side keeps small navigation controls: back, forward, and refresh. The existing proxy settings remain a single centered modal opened from the bottom-right settings button, avoiding the earlier duplicate-setting-button problem.

## Implementation Shape

The prototype uses static HTML with four persistent view panels. The real Rust/Wry version should use one native top shell plus multiple WebView instances, one per site, sharing the same WebView2 user data folder so login sessions persist.

The four sites should map to:

- ChatGPT: `https://chatgpt.com`
- Gemini: `https://gemini.google.com`
- NotebookLM: `https://notebooklm.google.com`
- Google AI Studio: `https://aistudio.google.com`

## Persistence

All WebViews should share the existing portable WebView2 profile directory under the executable `data` directory. This keeps the current local login persistence model. The active page key and started page list should be saved in app settings so the next launch restores the same workspace.

## Error Handling

If a page fails to load, the tab remains available and shows a retry action. Proxy status remains global because the embedded Clash runtime applies to the whole app WebView environment, not a single tab.

## Prototype

The static prototype is saved at `docs/prototypes/top-multipage-switcher-demo.html`.
