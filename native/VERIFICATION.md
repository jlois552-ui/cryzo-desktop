# Native desktop verification

Recorded 2026-10-28. These are local development checks, not a public release certificate.

## Current code checks

- The Tauri entry point imports the original OpenCode desktop renderer. Shared `packages/app`, `packages/ui`, `packages/session-ui`, and `packages/gui-plugins` sources remain unchanged. Branding is applied by the host build.
- The original interface passes `bun run typecheck:tsc` and `bun run build:ui` from `packages/desktop-tauri`.
- `cargo test --locked -p cryzo-engine -j1`: **14 passed**, zero failed (2 storage unit tests, 9 foundation integration tests, 3 native workflow integration tests).
- Workflow checks run an actual local HTTP provider fixture, stream a tool call, wait for approval, reject a reply for another session, write a real file after approval, reject approval reuse, restore saved messages after reopening SQLite, and cancel a pending write without effects.
- Retrying an already completed prompt preserves the original admission and does not start another model request.
- The terminal check starts a real Windows ConPTY process, reads its output, resizes it, and verifies that its attachment ticket cannot be reused.
- `script/verify-contracts.ts` decodes the captured native session, 4 messages, 19 durable events, and fixture catalogs with the unchanged upstream OpenCode schemas. No generated clients were edited.
- `cargo fmt --all --check` and `cargo clippy --locked -p cryzo-engine --all-targets -j1 -- -D warnings`: passed.
- `git diff --check`: passed.

Tests use temporary workspaces/databases and a local HTTP fixture. They make no paid model requests and do not write real provider credentials. `CRYZO_CONTRACT_CAPTURE` optionally saves the boundary capture; `CRYZO_UI_FIXTURE` optionally retains the actual completed workflow database for desktop inspection.

## Desktop build and interaction

- The Windows GNU desktop build succeeds with `cargo build --locked -p cryzo-desktop --features tauri/custom-protocol -j1`, development debug symbols disabled, and incremental compilation disabled for this laptop.
- The GNU linker reports `.rsrc merge failure: multiple non-default manifests`. MSVC release and manifest validation remain required before public distribution.
- The original interface loads, finishes onboarding, and opens a real local project. Unsupported SSH and WSL native adapters are reported as unavailable so the original startup gate can settle.
- The local desktop check uses a separate `CRYZO_DATA_DIR`; the standard app data directory is retained for the user's installed copy.
- No upstream TypeScript/Bun coding engine or Electron process is started by the Tauri executable.
- The corrected development executable is installed at `%LOCALAPPDATA%/Programs/CryzoDevelopment/Cryzo.exe`. The Desktop and Start menu `Cryzo Development` shortcuts point to it, and its original OpenCode home interface was observed running from that installed path on 2026-10-28. The prior executable is retained as `Cryzo.previous.exe`.
- The renderer currently requests an upstream v1 migration-status endpoint that the native engine does not implement, producing a transient 404 notification in the isolated desktop check. The installed copy also briefly reported a missing workspace path before settling on its empty home screen. These observations do not establish complete startup or upgrade parity.

## Remaining release gates

The implemented API-key Chat Completions path is not full baseline provider parity. Responses, Anthropic, Gemini, Bedrock, Azure-specific auth, subscription/OAuth, MCP, custom agents/skills, plugin main processes, compaction/reverts, session snapshots, media attachments and repository mutations need their own implementations and checks. Working-tree diff output does not establish session-specific snapshot support.

Live provider-account and OS credential-store interactions, macOS Apple Silicon/Intel, MSVC, signed installers, upgrade recovery, notarization and Store certification have not been certified by these checks. Earlier GitHub jobs were blocked by an account billing annotation; these results come from local builds.

Market, Team/Convex collaboration, Stripe billing and the landing page are later phases. No hosted feature or signed public download is claimed here. See `README.md` for the phase boundaries.


## Model connections, 2026-10-28

Implemented Cryzo Free's separate provider and native Google device authorization bridge; public integration contracts reuse the original OpenCode dialog. Personal API keys remain in the OS store, are masked in the UI, and are cleared after a successful connection. NVIDIA model discovery uses the live catalog; catalog discovery alone is not called credential verification.

Native transport fixtures cover Chat Completions, Responses, Messages, and Gemini generate-content. Text, reasoning, tool calls, replay metadata and secret-safe errors are exercised through real local HTTP responses. Model capabilities determine valid reasoning variants; Plan/Build selection preserves the model reference. Unknown pricing remains unknown. This does not establish live parity for every catalog entry, special cloud authentication or subscription OAuth.

The standalone Hono/Convex gateway enforces Google/JWT authentication, server-authorized transactional reservations, 100 daily UTC calls, one request per user, an account pool, idempotent step IDs and NVIDIA Retry-After. A new separate Convex development deployment was created and pushed successfully. Its real device endpoint rejects unknown clients and issues ten-minute codes for Cryzo. Google login completion and real inference remain unverified without Google OAuth credentials and the replacement NVIDIA key.

Vercel rejected creating the gateway project with HTTP 402: the selected connected account is suspended pending a payment method. No public gateway or signed desktop release is claimed. The previous website and its database were not changed.

Verification for this update: 19 native Rust tests, 6 gateway tests, and 5 model-selector unit tests passed. Engine clippy passed with warnings denied. The desktop type check covered 1,722 files; the preserved renderer production build succeeded. Windows GNU linking reports the pre-existing multiple-manifest warning; no signed distribution readiness is implied.

## Installed UI inspection, 2026-10-28

Windows Computer Use inspected the installed Cryzo process started at 14:40, after the updated build was installed. Home, New Session, the application menu, Preferences, Providers, Models and the integration search were opened. Searching NVIDIA returned Nvidia; searching Cryzo returned Cryzo Free. The separate running OpenCode application was inspected as a visual reference, including its populated Manage Models dialog. No OpenCode preferences were changed.

The installed Cryzo copy has no connected providers, its model settings show No model results, and the composer has no selectable model. Cryzo Free reported that it is not configured in this development build. The Providers settings page also displayed an empty Popular providers section, although Show more providers opens the searchable integration catalog. That empty settings section remains a UI observation needing investigation; it is not recorded as a passed provider-list check.

No real inference, credential validation, Google sign-in, reasoning-selection persistence through a real request, or end-to-end coding session was verified by this UI inspection. No secrets were entered and neither desktop application nor its server was restarted. Earlier transport-fixture results are separate from these installed UI checks.

## Global model settings and BYOK correction, 2026-10-28

The native Location handlers now treat `location[directory]=` as an omitted directory. The upstream client starts global Settings with this empty reference; canonicalizing it previously returned HTTP 400, which left Popular providers and Models unloaded. The existing HTTP contract test reproduced that 400 before the correction, then passed for global Location, Integration, Provider and Model requests after it.

NVIDIA and OpenRouter are prominent in the original connection picker, with NVIDIA also in Popular providers. The existing native API-key connection and four transport adapters are retained. Credential changes now emit the global `credential.switched` event expected by the upstream client so already-open project and Settings model catalogs refresh. The original credential-label editor now has its native PATCH implementation. Keys remain in the OS store; metadata responses contain external references.

The native provider test suite adds a metadata-only five-provider scenario for NVIDIA, OpenRouter, OpenAI, Anthropic and Google: connected model/provider catalogs populate, and renaming NVIDIA is reflected in its public connection record without returning a key. It does not contact live providers or write real credentials. Combined verification is 20 Rust tests, desktop type checking over 1,722 files, renderer build, and engine clippy with warnings denied. No universal provider or subscription-OAuth parity is claimed.

The user-supplied NVIDIA screenshot was read locally without printing or storing the key in source or logs. Small live MiniMax requests using the original and higher-resolution readings returned NVIDIA HTTP 403 with `Authorization failed`. No NVIDIA credential or success metadata was installed by those attempts. A text key or a replacement remains required to distinguish a transcription problem from a rejected credential.

## Cryzo Free gateway moved to Convex, 2026-10-28

The Hono/Vercel gateway was replaced by Convex HTTP actions on the existing `cryzo-desktop-models` development deployment (`moonlit-mosquito-262`). `/api/auth/*` (Better Auth), `/device`, `/v1/models`, `/v1/chat/completions` and `/health` are served from `https://moonlit-mosquito-262.convex.site`. Quota reservations are now internal mutations keyed by the Better Auth user resolved from the desktop's bearer session, so the shared `GATEWAY_QUOTA_SECRET` was removed from code and from the deployment. The desktop engine paths are unchanged; builds set `CRYZO_GATEWAY_URL` to the site origin.

Checks: `bunx tsc --noEmit` passed; `bun test` 6 passed (curated six-model catalog, request pinning, duplicate steps, per-user and pool limits, daily cap and Retry-After). Live: `/health` returned `configured:false` (expected until secrets are set); unauthenticated `/v1/models` and `/v1/chat/completions` returned 401; `/device` returned 200; the Better Auth device endpoint issued a code whose verification URI is the new `/device` page.

Not yet verified: `NVIDIA_API_KEY`, `GOOGLE_CLIENT_ID` and `GOOGLE_CLIENT_SECRET` are not set. The key transcribed from the user's screenshot again returned NVIDIA HTTP 403. Google sign-in completion, real inference through the gateway and the desktop model picker remain unverified.

The desktop was rebuilt with `cargo +stable-x86_64-pc-windows-gnu build --locked -p cryzo-desktop --features tauri/custom-protocol -j1` and `CRYZO_GATEWAY_URL=https://moonlit-mosquito-262.convex.site` (the default MSVC toolchain lacks Visual Studio build tools on this laptop). The executable contains the gateway origin and was installed to `%LOCALAPPDATA%/Programs/CryzoDevelopment/Cryzo.exe` for the next launch. Engine sources were not modified, so the Rust test suite was not re-run.

## Cryzo Free before sign-in, 2026-10-28

The six curated Cryzo Free models are now listed and enabled whenever the build has a gateway URL, before Google sign-in, like OpenCode's keyless free catalog. Personal-key models remain the default when connected. Prompting a free model without a sign-in for the configured gateway returns `InvalidRequestError` with `kind: "cryzo_sign_in"` before admission; the composer keeps the draft and opens the existing Connect Cryzo Free dialog.

Two pre-existing renderer blockers were fixed. `/api/config` was missing, so the composer never left its loading state and every send was held; it now returns the truthful empty document list. Models reported `time.released = 0`, which the renderer treats as old and hides by default; catalog release dates are now passed through, and each curated free model is its own family so all six are visible.

Checks: `tests/free_models.rs` added; 21 Rust tests passed, engine clippy passed with warnings denied, desktop type check passed. Installed UI (Windows Computer Use): the picker lists all six free models with Free badges, the composer shows Cryzo Free / DeepSeek V4.1 Flash, and sending opens the Google device-code dialog with the draft retained. Google sign-in completion and real inference remain unverified until `NVIDIA_API_KEY`, `GOOGLE_CLIENT_ID` and `GOOGLE_CLIENT_SECRET` are set on the gateway.

## Sign in with ChatGPT and provider list, 2026-10-29

Ported upstream's `chatgpt.ts` token-sharing sign-in to `engine/src/chatgpt.rs`: PKCE, a loopback callback, `dynamic_agent_client` registration with `agent_name_hint=Cryzo`, scope `chatgpt.tokens.use.direct`, ID-token claim checks (issuer, audience, nonce, expiry, subject), and refresh before each step. Only the refresh token is stored, in the OS credential store; the access token stays in memory. With ChatGPT connected, OpenAI lists only plan models, at zero per-token cost, and steps record zero cost.

OpenAI's ChatGPT backend streams responses with HTTP 200 and no `Content-Type`; the engine now accepts a missing content type and rejects only a different declared format, naming the HTTP status and provider error code without the body.

The Tauri host now implements `FilesOpenExternal` and `FilesOpenBrowser` (http/https, plus mailto for external), passing the parsed URL to the OS handler without a shell. Previously no link opened from the app.

OpenCode, OpenCode Go and GitHub Copilot are no longer offered. Provider wording follows the released OpenCode desktop ("Connect provider", "Other").

Checks: 23 Rust tests (2 new ChatGPT tests: offered method, authorize URL, forged-callback rejection, plan model list), engine clippy with warnings denied, desktop type check and build. Installed UI: Chrome opened OpenAI's "Use ChatGPT to sign in to Cryzo"; after approval, GPT-6.1 Sol replied in a Cryzo session, including after a restart (refresh path). Not yet implemented: zstd request compression, which ChatGPT requires for bodies over about 1 MiB.

## Session message order after restart, 2026-10-29

`GET /api/session/{id}/message` now honours `order`, `limit` (default 100, at most 200), `type` and opaque cursors. The desktop reopens a session with `order=desc` and reverses the page; the engine previously ignored the order and returned oldest first, so reopened timelines showed newest at the top. Cursors carry the order and the last message ID returned, so messages arriving between pages cannot shift them; an unknown cursor returns `InvalidCursorError`. `tests/messages.rs` failed before the change and passes after it. Verified in the installed app: a reopened ChatGPT session shows its messages oldest to newest. 24 Rust tests and engine clippy pass.

## Marketing tab, 2026-10-31

Postiz-style Marketing tab (`packages/app/src/market`): a rail with Calendar, Analytics and Media; a Channels column with Add Channel, Create Post, Write with Cryzo and grouped channels; week/day/month/list calendar with drag-to-reschedule; a post editor with per-channel versions, media, network settings, preview and delivery status; and a chat bar that opens a new session with a marketing brief and the local time. The Marketing button sits beside Search in the titlebar and in the vertical tab sidebar.

Backend: `services/model-gateway/convex/market` on the Cryzo cloud deployment, ported from cryzo.me's Composio publishing for X, Facebook, Instagram, YouTube, TikTok and Reddit, with Convex-scheduled publishing, leases and unknown-outcome protection. The engine proxies `/api/market/*` with the Cryzo sign-in token, streams uploads, and serves stored media as blobs, because the desktop content policy only allows `blob:` images. The agent gains `market_channels`, `market_posts` and `market_post`; scheduling needs exact-operation approval, and drafts do not.

Checks: gateway `bun run test` (6 quota and 6 marketing tests) and `tsc`; 26 Rust tests, including the marketing sign-in gate and date parsing; engine clippy with warnings denied; desktop type check and build. Installed UI: Marketing opened signed in, and the request reached Cryzo cloud, which reported `COMPOSIO_API_KEY is missing` as expected. Calendar week and month, Add Channel with six networks, Analytics, Media and the chat bar's prefilled session were all observed.

Not yet verified: connecting a real channel and publishing, which need the Composio environment values (`copy-composio-env.ps1`) and, for X and TikTok, custom auth configs. No live post was made.
