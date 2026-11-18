# Plan: move Marketing media from Convex storage to Cloudflare R2

**Status:** planned 2026-11-12. **Build only after the Meta, TikTok and YouTube reviews are approved.** This touches upload, publishing and the cryzo.me DNS, which the reviewers are exercising now.

## Why

Convex's free plan allows 1 GB of data egress a month. By Nov 12, Cryzo had used 820 MB, almost all of it on Nov 3–4 during the demo video uploads (`native/HANDOFF.md`, review status check). Every video post is paid for twice on Convex:

| Path | Convex cost |
|---|---|
| Desktop preview, or Meta/Instagram downloading the file from a `ctx.storage.getUrl` URL | **File serving** egress |
| `market/worker.publishDelivery` reading the file and sending its bytes to TikTok, YouTube, X or LinkedIn | **File serving** (read) + **fetch egress** (send) |

R2 charges nothing for egress. Storage is 10 GB free, then $0.015/GB-month. Reads (Class B) are 10M free per month and writes (Class A) are 1M free. Convex stays the backend for auth, quotas, posts, scheduling and tokens. Only the media bytes move.

## What changes for each network

| Network | Today | After |
|---|---|---|
| Facebook (photos `url`, videos `file_url`) | Meta pulls from a Convex URL | Meta pulls from `media.cryzo.me`: **free** |
| Instagram (`image_url` / `video_url`) | Meta pulls from a Convex URL | Meta pulls from `media.cryzo.me`: **free** |
| TikTok | `FILE_UPLOAD`: Convex reads and pushes the chunks | `PULL_FROM_URL` from a verified `https://media.cryzo.me/m/` prefix: **free**. `FILE_UPLOAD` stays as a fallback. |
| YouTube, X, LinkedIn | Convex reads and pushes | Convex reads from R2 (free) and pushes (fetch egress, **1× instead of 2×**) |
| Desktop previews (`GET media`) | Convex file serving | `media.cryzo.me`: **free** |

The remaining cost is the 1× push for YouTube/X/LinkedIn, at $0.132/GB on Convex Starter. Phase 4 removes it if volume ever justifies it.

## Architecture

```
desktop (Rust engine)
  1. POST media/upload-target  →  Convex: presigned R2 PUT URL + key   (no bytes through Convex)
  2. PUT file                  →  R2 S3 endpoint (single PUT, up to 5 GB; our cap stays 1 GB)
  3. POST media/register-r2    →  Convex: HEAD the object (size/type), insert marketMedia row
reads
  Convex signs  https://media.cryzo.me/m/<expiry>/<hmac>/<key>   (no network call)
  Cloudflare Worker "cryzo-media" checks the HMAC + expiry and streams from the R2 binding (Range supported)
```

- **Uploads go straight to R2's S3 endpoint with a presigned PUT, not through the Worker.** Workers on the free plan reject request bodies over 100 MB, and our upload cap is 1 GB. Presigned URLs only work on the S3 API domain, which is fine for uploads.
- **Reads go through the Worker on `media.cryzo.me`.** The signature sits in the **path**, not the query string, because some platforms dislike query strings. TikTok needs a stable, verified URL prefix and does not follow 3xx redirects, so the Worker must answer 200/206 itself. Previews get 1-hour URLs. URLs handed to networks get 24 hours, because Instagram and TikTok can process slowly.
- The bucket is **private**. No r2.dev public URL: Cloudflare says r2.dev is rate-limited and not for production.

## Steps

### Phase 0: accounts and DNS (needs the user; do after reviews)
1. The user creates or signs in to a Cloudflare account. I never create accounts or enter passwords.
2. **Move cryzo.me's nameservers from Google to Cloudflare (free plan).** Both a Worker custom domain and an R2 custom domain need the zone on Cloudflare.
   - Before switching, copy every existing record: the Vercel A/CNAME records, the Google Search Console TXT record, TikTok/Meta verification TXT records, and MX records if any.
   - Switch at the registrar. The user does this, because the change is hard to reverse.
   - Then check that `https://www.cryzo.me/privacy`, `/terms`, `/data-deletion`, `/download` and `/api/market/oauth/x/callback?state=bogus` still behave.
   - Fallback if they won't move DNS: a `cryzo-media.<account>.workers.dev` hostname, verified for TikTok with a URL-prefix signature file served by the Worker. It works, but looks less professional.
3. Create bucket `cryzo-media` (location hint: ENAM). Add lifecycle rules: delete `tmp/` after 1 day. Optionally delete `u/` objects with no post after 90 days; that needs a Convex sweep, since R2 can't see the database.
4. Create an R2 API token scoped to **this bucket only**, with Object Read & Write.

### Phase 1: Worker `cryzo-media` (new folder `services/media-worker`)
- `wrangler.toml`: an R2 binding `MEDIA` → `cryzo-media`, route `media.cryzo.me/*`, and secret `MEDIA_SIGNING_KEY` set via `wrangler secret put`. The user runs it so the value is entered at a hidden prompt.
- `GET /m/<exp>/<sig>/<key>`:
  - reject an expired or bad HMAC with 403, using a constant-time compare;
  - `MEDIA.get(key, { range, onlyIf })`;
  - stream the body with `Content-Type`, `Content-Length`, `Accept-Ranges` and `ETag`, and answer 206 for ranges;
  - `Cache-Control: private, max-age=3600`.
- `GET /m/tiktok-verify.txt` (or whatever file TikTok issues): only if the DNS TXT method isn't used.
- Tests use `@cloudflare/vitest-pool-workers` with a local R2: signature valid, expired or tampered; Range; missing object; HEAD.

### Phase 2: Convex (`services/model-gateway`)
- Env: `R2_ACCOUNT_ID`, `R2_BUCKET`, `R2_ACCESS_KEY_ID`, `R2_SECRET_ACCESS_KEY`, `MEDIA_SIGNING_KEY`, `MEDIA_BASE_URL=https://media.cryzo.me`, `MEDIA_BACKEND=r2|convex`.
  - Secrets go in through a `-Prod` hidden-prompt script, the same pattern as `set-market-secrets.ps1`.
  - The signing key is generated and piped straight into `convex env set --prod` and `wrangler secret put`, never printed.
- `lib/media.ts` (new):
  - `presignPut(key, contentType, size)`: SigV4 using WebCrypto, or `aws4fetch` if it runs in the Convex runtime.
  - `head(key)`, `remove(key)`.
  - `signedUrl(key, ttl)`: HMAC-SHA256 over `exp/key`, base64url.
  - Unit-test the presign against the AWS SigV4 test vectors.
- Schema:
  - `marketMedia` gains `r2Key: v.optional(v.string())`, and `storageId` becomes optional (old rows keep working).
  - `marketPosts.mediaIds` switches from `v.id("_storage")` to `v.id("marketMedia")`. A one-off migration maps existing posts (a handful of rows) by looking up `by_storage`.
- HTTP (`market/http.ts`): add `media/upload-target` and `media/register-r2`. **Keep `media/upload-url` and `media/register` working** for desktop builds already in users' hands, until they update.
- `listMedia` and `deliveryForPublish`: `url = row.r2Key ? signedUrl(row.r2Key, ttl) : ctx.storage.getUrl(row.storageId)`.
- `deleteMedia`: R2 delete for R2 rows. Deleting needs an action, or a scheduled action from the mutation.
- `lib/providers/tiktok.ts`:
  - when `TIKTOK_PULL=1` and the media URL starts with `MEDIA_BASE_URL`, send `source_info: { source: "PULL_FROM_URL", video_url }`;
  - otherwise keep `FILE_UPLOAD`;
  - add a test for each branch.
- Tests: `bun run test` and `bunx tsc --noEmit`. No real R2 or network calls in tests; use local HTTP fixtures, per `native/AGENTS.md`.

### Phase 3: desktop engine (`native/engine/src/market.rs`)
- Upload:
  - `media/upload-target` → `{ url, key, headers }`;
  - `PUT` the file stream with `Content-Type` and `Content-Length`, the same 1 GB cap and the same 30-minute timeout;
  - `media/register-r2 { key, name, size, contentType }`;
  - return `{ data: { mediaId } }`, keeping `storageId` in the response for the UI until it switches over.
- When `upload-target` returns 404 (an older cloud), fall back to the old flow, so the two can ship in either order.
- Run `cargo fmt`, `cargo clippy --all-targets -- -D warnings` and `cargo test`. Add a test against a local HTTP fixture standing in for R2 and the gateway.
- Rebuild and reinstall the desktop (`native/install-development.ps1`), then cut a new preview release so users get it.

### Phase 4 (only if the numbers say so): push without Convex egress
YouTube, X and LinkedIn need the bytes pushed. If Convex fetch egress from those ever costs real money, a second Worker route (`POST /push`, HMAC-authenticated) could stream from R2 to the network's upload URL. Outgoing fetches from a Worker aren't capped like incoming bodies. It would get the short-lived network upload URL, but **never** the OAuth refresh token. Skip it until the bill is over about $10/month.

## Rollout
1. Phase 0 + 1: deploy the Worker. Check a signed URL to a test object in the browser, and Range with `curl -r`.
2. Phase 2 with `MEDIA_BACKEND=convex`: deploy and confirm nothing changed.
3. Phase 3 desktop build: upload a test image and video, and check that previews load from `media.cryzo.me`.
4. Set `MEDIA_BACKEND=r2`. Publish test posts, each Private or "Only me" with the user's OK:
   - one Facebook photo, then one Instagram image and one Reel;
   - one YouTube Private video;
   - one TikTok "Only me", first with `FILE_UPLOAD`, then after verifying the `https://media.cryzo.me/m/` prefix in the TikTok portal, with `TIKTOK_PULL=1`.
5. Watch Convex Usage → Data Egress for a week. File Serving should go to ~0, and Fetch Egress should only come from YouTube/X/LinkedIn.
6. Later, with the user's OK: delete leftover test media from Convex storage (413 MB on Nov 12).

**Rollback:** set `MEDIA_BACKEND=convex`. Old rows and old clients keep working throughout.

## What the user does
- Create or sign in to Cloudflare, and approve `wrangler login`.
- Switch cryzo.me's nameservers at the registrar. I prepare the exact record list first.
- Paste the R2 token secret into the hidden prompt, once.
- Approve each test post, and the final cleanup of old test media.

## Sources (checked 2026-11-12)
- R2 pricing: https://developers.cloudflare.com/r2/pricing (10 GB-month storage, 1M Class A and 10M Class B free; egress free)
- R2 public buckets and r2.dev limits: https://developers.cloudflare.com/r2/buckets/public-buckets/
- TikTok media transfer guide (PULL_FROM_URL, URL properties, no redirects): https://developers.tiktok.com/doc/content-posting-api-media-transfer-guide
- Convex pricing (egress $0.132/GB on Starter): https://www.convex.dev/pricing
