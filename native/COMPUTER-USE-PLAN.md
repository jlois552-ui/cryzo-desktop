# Cryzo computer use: build plan

Status (2026-11-11): Phase 1 built, Phase 2 settings built (UI parts pending), Phase 3 engine side built. See HANDOFF.md "Computer use".
Phase 0 (local-model testing) skipped by decision: test with Claude instead.

## Goal

Let an agent operate the user's own desktop apps by **reading the screen as text** instead of
screenshots: Windows UI Automation (UIA) first, OCR as the fallback, screenshots last. Text is
faster and cheaper, and it works with **text-only local models** (Ollama, LM Studio) that cannot
read images.

Ship it two ways from one Rust crate:

1. **A standalone MCP server** (`cryzo-desktop-mcp`) that any MCP client can use: Claude, Codex,
   Cursor, or a local-model client.
2. **A built-in Cryzo tool**, behind the engine's existing exact-operation approvals.

## Why this is open

| Product | What it does |
|---|---|
| OpenCode (official) | No computer use. A third-party Mac-only "Superapp" adds it. |
| Cline | Browser only, a screenshot after every action. |
| Cursor | Built-in browser, cloud computer use, reportedly desktop control (Oct 2026); screenshot-based. |
| Devin | Computer use on its own cloud Linux desktop, not the user's machine. |
| Claude computer use | Screenshots; browsers are view-only. |

Open-source pieces already prove the text-first approach on Windows: **Windows-MCP** (Python,
MIT, UIA tree with element IDs, 0.2–0.5 s per action), **FlaUI-MCP** (.NET), **dsh-cua**
(accessibility first, screenshot fallback). None is built into an agent app, and none targets
small local models deliberately.

## Phase 0: test the idea before building (half a day)

Cryzo's native engine has no MCP client yet, so test from outside:

- Run `uvx windows-mcp serve` and connect it to Claude Code, and to a local model through an MCP
  client (for example LM Studio with a 7–14B Qwen or Llama).
- Run the 10 tasks in "Evaluation" below. Record success, steps, seconds per step, and tokens per
  snapshot.
- Decision: if a local model succeeds on at least half the tasks from text alone, build Phase 1.
  If snapshots are too large, the snapshot format (Phase 1b) is the real product.

## Phase 1: `cryzo-desktop` crate and MCP server (Windows)

New crate `native/desktop-control` (library) plus a thin `cryzo-desktop-mcp` binary (stdio MCP).
Windows only first; the code is behind `cfg(windows)`, with clean "unsupported" errors elsewhere.

### 1a. Reading

- **UIA tree** via the `uiautomation` crate (Apache-2.0, actively maintained; pin the version
  because releases break often). Walk the foreground window, or a named window, with a time and
  node budget. Skip offscreen and disabled subtrees.
- **Browsers and Electron/WebView2 apps:** Chromium builds its full web accessibility tree only
  when it thinks a screen reader is present. UIA clients normally trigger this; document
  `--force-renderer-accessibility` as the fix when a page shows up empty.
- **OCR fallback** with Windows' built-in `Windows.Media.Ocr` (through the `windows` crate). It is
  local and free, with no model download. It runs on a window or region capture only when UIA returns
  nothing useful: games, canvases, video, remote desktops.
- **Screenshot** of a window or region on explicit request, for genuinely visual questions.

### 1b. Snapshot format (the part that matters for local models)

A compact, stable text form, for example:

```
window "Cryzo" (Cryzo.exe) 1280x800
  [e12] button "Create post"
  [e13] tab "Calendar" selected
  [e27] edit "What do you want to share?" value=""
  [e31] list "Post to" (3 items)
    [e32] checkbox "Alu (YouTube)" checked
```

- Short IDs, valid until the next snapshot. Include only the role, the name, the value, and the
  state (checked, selected, disabled, focused).
- A token budget per snapshot (default about 1,500 tokens). Large lists collapse to "N more".
  `expand e31` reveals them.
- **Diff mode:** after an action, return only what changed. This is the main speed and cost win.
- Password fields never return their values.

### 1c. Acting

- Prefer UIA patterns over pixels: `Invoke` (click), `ValueSetter` (type into a field),
  `Toggle`, `SelectionItem`, `ExpandCollapse`, `Scroll`.
- Fall back to `SendInput` mouse and keyboard at the element's centre only when no pattern
  exists. Use OCR coordinates only when OCR found the target.
- `wait_for(text | element | window, timeout)` instead of fixed sleeps.

### 1d. MCP tools

| Tool | Purpose |
|---|---|
| `windows` | List top-level windows (title, app, ID). |
| `snapshot(window?, diff?)` | Compact UIA text tree (1b). |
| `find(text, role?)` | Search the current tree; returns IDs. |
| `act(id, action, value?)` | click, type, toggle, select, expand, scroll, focus. |
| `key(chord)` | Keyboard shortcut to the focused window. |
| `ocr(window?, region?)` | Text plus boxes when UIA is empty. |
| `screenshot(window?, region?)` | Image, last resort. |
| `wait_for(...)` | Poll until text, element or window appears. |
| `launch(app)` / `focus(window)` | Start or switch apps. |

No shell, registry or file tools; Cryzo already has those with their own approvals.

## Phase 2: safety (ships with Phase 1, not after)

Safety limits are **user settings with safe defaults**, not hard limits (decided 2026-11-11):
power users can loosen them, new users start safe.

- **Off by default.** Enabled per session, with a visible "Cryzo is controlling your computer"
  banner and a global stop hotkey (Ctrl+Alt+Esc) that ends control immediately. These two stay on.
- **Approval mode setting:** "ask every time" (default), "ask once per app", or "don't ask".
  Reading (`windows`, `snapshot`, `find`, `ocr`) never needs approval. Acting goes through the
  engine's existing exact-operation approval when asking is on.
- **"Type into password fields" setting:** off by default; the user can turn it on.
- **Blocked apps setting:** pre-filled (banking, password managers, Windows Security), fully
  editable, can be emptied.
- **Hard limit (Windows itself):** elevated/UAC windows cannot be controlled from a normal app.
- Agents still apply their own policies on top (for example, Claude will not type passwords or
  move money even when the tool allows it).
- **Prompt injection:** on-screen text is data. The agent's system prompt and tool results label
  it so; instructions found on screen are never followed without the user.
- **Audit log** of every action (app, element name, action), local only, no screen contents.
- **No telemetry** (Windows-MCP enables telemetry by default; Cryzo's will not exist).

## Phase 3: inside Cryzo

- Engine: add `Operation::Desktop { … }` in `engine/src/tools.rs`, with `needs_approval()` true
  for acting operations. That reuses `request_approval` / `consume_approval` in `store.rs`, so an
  approval binds to the exact action.
- Desktop UI: approval cards that show "Click 'Post now' in Chrome", the control banner, the stop
  button, and a Settings toggle (default off).
- Works with any model Cryzo can talk to, including local ones, because nothing requires vision.

## Phase 4: macOS and Linux

The same crate API on the macOS Accessibility (AX) API, with Apple's Vision framework for OCR, and
on Linux AT-SPI with Tesseract for OCR. Only the reading and acting layers change; the snapshot
format, tools and safety stay the same.

## Evaluation (run in Phase 0, then on every change)

Ten tasks, scored for success, steps, seconds and tokens, each with a local 7–14B model and with
Claude:

1. Open Notepad, type a line, save to Desktop.
2. In Cryzo, open Marketing → Calendar and read the next scheduled post.
3. Change a setting in Windows Settings (dark mode) and change it back.
4. Find a file in File Explorer by name and open it.
5. Fill a web form in Chrome through the UIA tree.
6. Read an email subject in Outlook.
7. Rename a layer in an Electron app.
8. Read text from a game or canvas window (OCR path).
9. Recover when a dialog pops up mid-task.
10. Stop instantly when the user presses the stop hotkey.

## Risks

- Some apps have slow or huge accessibility trees. Mitigate with time and node budgets, a
  per-process skip list, and diff mode.
- Some apps expose nothing useful (custom-drawn UIs). OCR covers reading; acting falls back to
  coordinates.
- Small local models still make mistakes. Approvals and the stop hotkey matter more than accuracy.
- Licensing: Windows-MCP is MIT and the `uiautomation` crate is Apache-2.0, so borrowing ideas or
  code with attribution is fine. Do not copy AGPL code.

## Rough effort

| Phase | Effort |
|---|---|
| 0: test with Windows-MCP | half a day |
| 1 + 2: crate, MCP server, safety (Windows) | 1–2 weeks |
| 3: Cryzo integration | about 1 week |
| 4: macOS and Linux | 1–2 weeks each |
