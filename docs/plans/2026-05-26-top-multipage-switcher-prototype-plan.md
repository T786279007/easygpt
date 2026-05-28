# Top Multipage Switcher Prototype Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build a static HTML prototype that demonstrates a top switcher for ChatGPT, Gemini, NotebookLM, and Google AI Studio with persistent in-window page views.

**Architecture:** The prototype is a standalone HTML file under `docs/prototypes`. It simulates multiple started pages by keeping one DOM panel per site and toggling visibility, which mirrors the intended Rust/Wry multi-WebView behavior without depending on remote sites that block iframe embedding.

**Tech Stack:** HTML, CSS, vanilla JavaScript.

---

### Task 1: Static Prototype

**Files:**
- Create: `docs/prototypes/top-multipage-switcher-demo.html`

**Step 1: Create the HTML shell**

Add a top application bar, four site buttons, navigation icon buttons, a main page surface, and a centered settings modal.

**Step 2: Add persistent page panels**

Create one panel for each site and keep them mounted after first activation. Hide inactive panels with CSS instead of rebuilding them.

**Step 3: Add JavaScript state**

Track `activeSite`, `startedSites`, and per-site note text in memory. Update tab states and page content on click.

**Step 4: Verify manually**

Open the file in a browser, switch through all four page buttons, type into a page note, switch away and back, and confirm the note remains.

**Step 5: Screenshot review**

Use Browser or Playwright to capture the prototype and check that the top bar, page area, and settings modal do not overlap.
