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

The development deployment is `moonlit-mosquito-262` in the `cryzo-desktop-models` project.

## Behavior

- Better Auth owns accounts, sessions and RFC 8628 device authorization. The user signs in with Google in their browser, confirms the desktop's code, then approves or denies. Device requests expire after ten minutes; polling has a five-second minimum. Desktop tokens stay in the OS credential store.
- The local auth component extends the pinned package's unmodified `baseSchema.ts` with the device plugin table and indexes.
- Only six curated models that appear in NVIDIA's current catalog are offered. Full GLM 5.3 is distinct from GLM 5.3 Flash. No paid fallback or upstream subsidized offers are inherited.
- Each admitted model step consumes one of 100 UTC daily calls, including failed upstream attempts. Duplicate step IDs cannot invoke NVIDIA again. Defaults: one active generation per user, two across the account, 20 requests per minute. The gateway's 270-second NVIDIA timeout is shorter than its 330-second quota lease. A model that sends no response within 60 seconds is reported as temporarily unavailable and releases the user's slot; `bunx convex run diagnose:models` reports each curated model's current status and latency.
- Atomic Convex mutations enforce limits. Quota mutations are internal: only the gateway's HTTP actions can call them, after resolving the Better Auth bearer session. NVIDIA Retry-After blocks the shared pool temporarily.
- Prompts, tool arguments, credentials and provider bodies are not logged. Model messages and tool schemas are sent to NVIDIA; tools execute locally under native approvals.
- NVIDIA API trial terms restrict production use; production entitlement/capacity remains a rollout dependency. This code makes no purchases and does not enable a public desktop release.

## Verification

Run `bun test`, `bunx tsc --noEmit`, and `bunx convex dev --once`. Before public access verify Google approval/denial, expired tokens, device-code polling, current model availability, cancellation, quotas and upstream errors. `/health` reports configuration presence, not verified Google or NVIDIA access.
