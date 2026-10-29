# Cryzo native implementation

This is the native implementation branch of [Cryzo Desktop](https://github.com/Imma2013/cryzo-desktop), forked from OpenCode. It is **an internal development build, not a public alpha**.

The Tauri frontend imports the original OpenCode desktop renderer, components, and styles directly. Cryzo's desktop executable embeds the Rust engine directly. It does not start Electron, Bun, the upstream TypeScript engine, or a browser sandbox. The implemented native coding path is described below; full upstream capability parity is a separate release gate.

## Run

Prerequisites: Bun 1.4.4, current stable Rust, and the [Tauri platform prerequisites](https://v2.tauri.app/start/prerequisites/). Windows requires Visual Studio C++ build tools and the Windows SDK. macOS requires Xcode command line tools.

```sh
bun install --frozen-lockfile --filter '@cryzo/desktop' --filter '@opencode/app' --ignore-scripts
bun run --cwd packages/desktop-tauri dev
```

Engine checks are independent of the frontend:

```sh
cd native
cargo fmt --all --check
cargo clippy -p cryzo-engine --all-targets -- -D warnings
cargo test -p cryzo-engine
```

Run the standalone development HTTP service with `cargo run -p cryzo-engine -- serve <data-directory>`. Its first stdout line contains the loopback URL and a process-specific Bearer token. Treat that line as a credential; do not publish it. The Tauri host starts its own in-process loopback service and proxies renderer requests over IPC; its token stays in Rust. A separate CLI service is not required.

## Implemented in this branch

- Tauri 2 host with the upstream SolidJS desktop renderer, Cryzo name/icon, and a native platform adapter. The shared UI layout and styles are preserved; provider connection behavior is adapted to the native engine.
- Durable native settings and drafts, including strict attachment-reference checks, plus native folder/file pickers and scoped attachment reads.
- Original session, message, inbox, agent/model selection, permission, filesystem, Git diff, project and event contracts for the implemented coding path.
- A native coding runner with real provider streaming, tool calls, build/plan modes, exact approval cards, interruption, committed history between model steps, and durable restart recovery without replaying effects.
- Interactive native PTYs with real shell input, resize, Unicode output, reconnect cursors, and one-use attachment tickets. The existing terminal UI communicates with them over Tauri IPC.
- Supported API-key Chat Completions provider/model catalogs. Saved credentials are OS-store references; unsupported native transports are not offered as connected models.
- OS credential storage, not browser storage or SQLite secrets.
- SQLite WAL sessions, idempotent prompt admission, ordered resumable event pages, and explicit interrupted-session status after restart.
- An OS file lock keeps a second local engine from resetting an active owner's recovery state.
- Authenticated loopback-only development HTTP service and direct Tauri IPC.
- Workspace-relative file reads, listings and writes, Git diffs, and bounded-duration program execution.
- Single-use approval bound to the exact session and operation for writes and commands.
- API-key Chat Completions streaming with cancellation and network-fragment-safe SSE decoding.
- Frozen OpenCode baseline: 230 provider entries and 8,787 model entries. This is an inventory, **not verified provider parity**.
- Native contract tests and CI configuration.

## Model connections update

The preserved connection screens now expose Cryzo Free as a separate Google device login integration, alongside personal keys. The Rust engine supports Chat Completions, OpenAI Responses, Anthropic Messages and Gemini generate-content, including streamed reasoning, tool calls and provider replay data. These are fixture-verified transport families; live compatibility for every catalog entry is not implied. Provider-specific authentication and subscription OAuth gates still apply.

NVIDIA personal-key connections reconcile with the live catalog. Public catalog access does not verify a credential. Unknown prices are omitted, reasoning variants follow each model's capabilities, and Plan/Build switches retain the selected model and effort. Key inputs are masked and cleared after connection.

Cryzo Free routes through `services/model-gateway`, with Google device login and 100 calls per UTC day per user. Set `CRYZO_GATEWAY_URL` to the gateway's Convex site origin (for development, `https://moonlit-mosquito-262.convex.site`) when building or running the desktop; an unconfigured build reports unavailable. The owner NVIDIA key stays exclusively on the server. See the service README for configuration and remaining live setup requirements.

## Work remaining before public alpha

1. **Remaining engine parity:** compaction, full steering/queue controls, custom agents/skills, MCP/OAuth, plugin compatibility, repository/worktree mutation, session-specific snapshots and reverts, media attachments, advanced search, and the remaining upstream API/event contracts. Worktree inventory and working-tree diffs are available; full repository lifecycle and per-session snapshots are not implied.
2. **All baseline providers:** transport/auth adapters and fixtures for Responses, Anthropic, Gemini, Bedrock, Azure, Copilot and all other baseline integrations. API-key Chat Completions code alone does not establish provider compatibility. Verify OpenRouter free-model limits and NVIDIA NIM using each user's own account. Do not inherit OpenCode's subsidized model offers.
3. **Production verification:** complete platform and UI interaction coverage for the preserved OpenCode application, including unsupported capability handling, macOS window behavior, accessibility, upgrades, and signing. The old inspector is not the product UI.
4. **Market:** a new Convex project for account connections, drafts/assets, calendar, approvals, scheduled jobs, delivery attempts and reconciliation. Hosted workers must publish while desktop is closed. Connect Facebook, Instagram, TikTok, YouTube, LinkedIn, Google Business Profile and Pinterest through approved provider APIs. Use Composio where its supported tool contracts cover the required operation. Do not copy Postiz AGPL code into this MIT project.
5. **Team:** organizations, categories/channels, roles, DMs, threads, attachments with a file-size cap, names and membership checks. Selected messages can become a task; the chosen local executor must accept it before creating an isolated worktree. Chat messages must never directly execute commands. Voice/video are later work.
6. **Billing:** Stripe subscription entitlements and a credit ledger together; signed webhooks and idempotent grants. Final prices and allowances require measured operating costs and owner approval.
7. **Distribution:** develop, test and build locally. Windows launches through the Microsoft Store; macOS Apple Silicon and Intel downloads are distributed through GitHub Releases after signing and notarization. GitHub-hosted builds are not a development prerequisite. Windows Store work still requires a Partner Center listing, a signed offline-capable installer at an immutable HTTPS download URL, update handling and Store certification. Verify provenance and upgrade/recovery on both platforms. No public download until signatures and parity gates pass.
8. **Web:** Cryzo landing page, docs and download links on Vercel, connected only to verified releases. Keep the previous Cryzo website/database/deployment separate.

Each feature must be working and verified before it is shown as available. Do not replace unfinished integrations with success responses, placeholder controls, or fallback calls into the old engine.

## Local app and hosted collaboration

Local development and distribution do not change the backend architecture. The Tauri app runs the Rust coding engine, terminal and repository tools on the user's computer. A separate hosted Convex project owns team accounts, membership, channels, messages, threads and shared task status. The SolidJS interface can use Convex's framework-independent reactive client for authenticated subscriptions. Permit only the configured deployment's HTTPS and WebSocket endpoints in the desktop CSP when that integration is implemented; do not expose an admin/deploy key to the app.

Messaging and shared status require an internet connection. Local files, history and coding tools remain local; model requests still require connectivity unless a local model is selected. A team task needs acceptance on its chosen executor before it can run local tools. Convex subscriptions provide live application data; simultaneous editing of the same source file needs a separately implemented editing protocol and is not implied by messaging support. Hosted marketing jobs continue independently of whether the desktop app is open.

For the Windows Store route, `tauri.windows.conf.json` uses the offline WebView2 installer. Tauri's documented Win32 Store route accepts a signed NSIS installer with `/S` as the silent-install argument. Hosting its versioned installer for Store retrieval is separate from the public macOS GitHub download channel. These settings prepare packaging; they do not reserve a Store product, configure signing, or certify a release.

## Source and licensing

Baseline commit and catalog digest are recorded in `baseline.json`. `node native/freeze.mjs` regenerates the inventory from the checked-in upstream snapshot. Review any baseline change intentionally.

The original repository's MIT license and notices remain intact. New Cryzo code is MIT. The OpenCode name/assets in the upstream tree are reference material, not a claim of affiliation. Never copy Postiz code without separately resolving its AGPL obligations.
