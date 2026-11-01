# Cryzo Free model gateway

Separate from the paused website. OpenCode's desktop UI is preserved; its Rust agent, files, terminal and tool approvals remain local. This Convex service handles Google authentication, quotas and NVIDIA streaming.

## Development and deployment

The whole gateway runs on Convex: Better Auth routes (`/api/auth/*`), the device page (`/device`), `/v1/models`, `/v1/chat/completions` and `/health` are HTTP actions in `convex/http.ts` and `convex/gateway.ts`. There is no separate web host.

Run `bun install`, then `bunx convex dev` from this directory (`bunx convex dev --once` pushes once). Generated bindings are checked in. Deploy production with `bunx convex deploy`.

Set these on the Convex deployment with `bunx convex env set NAME value`:

| Variable | Value |
| --- | --- |
| `SITE_URL` | The deployment's `https://<name>.convex.site` origin |
| `BETTER_AUTH_SECRET` | Random 32 bytes |
| `GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET` | Google Cloud web OAuth client with redirect URI `${SITE_URL}/api/auth/callback/google` |
| `NVIDIA_API_KEY` | Owner key from build.nvidia.com |

Build the desktop with `CRYZO_GATEWAY_URL` set to the same `SITE_URL`. No Google client secret or NVIDIA key belongs in the desktop bundle.

The development deployment is `moonlit-mosquito-262` in the `cryzo-desktop-models` project. Production is `benevolent-rhinoceros-399`; use `--prod` with Convex CLI and build production desktop updates with `CRYZO_GATEWAY_URL=https://benevolent-rhinoceros-399.convex.site`. Keep Google sign-in (`cryzo-sign-in`) separate from the YouTube Marketing OAuth project (`cryzo-desktop`).

## Behavior

- Better Auth owns accounts, sessions and RFC 8628 device authorization. The user signs in with Google in their browser, confirms the desktop's code, then approves or denies. Device requests expire after ten minutes; polling has a five-second minimum. Desktop tokens stay in the OS credential store.
- The local auth component extends the pinned package's unmodified `baseSchema.ts` with the device plugin table and indexes.
- Only six curated models that appear in NVIDIA's current catalog are offered. Full GLM 5.3 is distinct from GLM 5.3 Flash. No paid fallback or upstream subsidized offers are inherited.
- Each admitted model step consumes one of 100 UTC daily calls, including failed upstream attempts. Duplicate step IDs cannot invoke NVIDIA again. Defaults: one active generation per user, two across the account, 20 requests per minute. The gateway's 270-second NVIDIA timeout is shorter than its 330-second quota lease. A model that sends no response within 60 seconds is reported as temporarily unavailable and releases the user's slot; `bunx convex run diagnose:models` reports each curated model's current status and latency.
- Atomic Convex mutations enforce limits. Quota mutations are internal: only the gateway's HTTP actions can call them, after resolving the Better Auth bearer session. NVIDIA Retry-After blocks the shared pool temporarily.
- Prompts, tool arguments, credentials and provider bodies are not logged. Model messages and tool schemas are sent to NVIDIA; tools execute locally under native approvals.
- NVIDIA API trial terms restrict production use; production entitlement/capacity remains a rollout dependency. This code makes no purchases and does not enable a public desktop release.

## Marketing

`convex/market/` serves the desktop's Marketing tab under `/market/*`, behind the same Better Auth session as Cryzo Free. The desktop engine forwards these requests with the token from the OS credential store.

- `store.ts`: accounts, groups, posts, deliveries, media and the analytics cache. Every function takes the user resolved by the HTTP layer.
- `lib/providers/`: one module per network (X, LinkedIn, Facebook, Instagram, YouTube, TikTok, Reddit) with its OAuth sign-in, token refresh, profile, Page or account choice, publishing and analytics, called directly over each network's REST API. The flow follows Postiz's integrations; no Postiz code is used.
- `worker.ts` (Node): finishes sign-ins, refreshes tokens ten minutes before expiry, publishes and reads analytics. A post is scheduled with `ctx.scheduler.runAt`. Each network gets one delivery with a 5-minute lease; a dispatch that never reports back becomes `unknown` and is never retried automatically, to avoid duplicate posts. TikTok publishing is confirmed by polling its status.
- `http.ts`: the routes. `POST /market/accounts/connect` returns the network's sign-in URL; the browser comes back to `/market/oauth/<network>/callback`, where a single-use, 10-minute state row identifies the user. `GET /market/providers` lists which networks are set up and which wait on platform approval.

Tokens are sealed with AES-256-GCM (`lib/crypto.ts`, key `MARKET_TOKEN_KEY`) and never returned to clients. Changing the key disconnects every channel. Facebook, Instagram and LinkedIn post as a chosen Page, business account, member or organization; when a sign-in finds exactly one, it is chosen automatically. Facebook, Instagram and LinkedIn (without partner refresh tokens) cannot refresh, so those channels show Reconnect after about 60 days.

Configuration (Convex env), set with `.\set-market-secrets.ps1` (hidden prompts; pass `-Network x` for one network). It generates `MARKET_TOKEN_KEY` on first run and prints the callback URLs to register:

| Network | Env | Callback path |
|---|---|---|
| X | `X_CLIENT_ID`, `X_CLIENT_SECRET` | `/market/oauth/x/callback` |
| LinkedIn | `LINKEDIN_CLIENT_ID`, `LINKEDIN_CLIENT_SECRET` | `/market/oauth/linkedin/callback` |
| YouTube | `YOUTUBE_CLIENT_ID`, `YOUTUBE_CLIENT_SECRET` | `/market/oauth/youtube/callback` |
| TikTok | `TIKTOK_CLIENT_KEY`, `TIKTOK_CLIENT_SECRET` | `/market/oauth/tiktok/callback` |
| Facebook, Instagram | `META_APP_ID`, `META_APP_SECRET` | `/market/oauth/facebook/callback`, `/market/oauth/instagram/callback` |
| Reddit | `REDDIT_CLIENT_ID`, `REDDIT_CLIENT_SECRET` | `/market/oauth/reddit/callback` |

`MARKET_PENDING` lists networks or features still waiting on platform approval (currently `reddit,linkedin-page,youtube,tiktok`); the desktop shows them as pending, and LinkedIn requests organization scopes only once `linkedin-page` is removed. Platform review status and demo-recording scripts are in `native/MARKET-REVIEW.md`.

Production sets `MARKET_CALLBACK_BASE=https://www.cryzo.me/api/market`. Register that website base plus `/oauth/<network>/callback`, not the deployment-local fallback paths in the table, in production provider portals. The website callback proxy forwards to the production gateway. Its policies are `/privacy`, `/terms` and `/data-deletion` on `https://www.cryzo.me`.

Credentials being present does not establish platform approval or a connected account. `/health` checks gateway configuration only; use authenticated `/market/providers`, the Channels screen, real OAuth returns and permitted read operations for separate verification. See `../../native/MARKET-REVIEW.md` for the current production setup and exact remaining blockers. Never publish a real post during verification without explicit approval for that post.

`bun run test` runs the quota and marketing suites in separate processes: Convex scheduled-function timers from one suite must not run into the other.

## Verification

Run `bun test`, `bunx tsc --noEmit`, and `bunx convex dev --once`. Before public access verify Google approval/denial, expired tokens, device-code polling, current model availability, cancellation, quotas and upstream errors. `/health` reports configuration presence, not verified Google or NVIDIA access.
