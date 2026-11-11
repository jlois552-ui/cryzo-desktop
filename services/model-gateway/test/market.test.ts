import { linkedin } from "../convex/lib/providers/linkedin"
import { instagram } from "../convex/lib/providers/meta"
import { tiktok } from "../convex/lib/providers/tiktok"
import { afterEach, describe, expect, spyOn, test } from "bun:test"
import { convexTest } from "convex-test"
import schema from "../convex/schema"
import { internal } from "../convex/_generated/api"
import { challenge, open, seal } from "../convex/lib/crypto"
import { graphMetrics, publishProblem } from "../convex/lib/market"

// The network worker is replaced by no-ops: scheduled publishing and refresh runs must not reach the network.
const modules = {
  "../convex/market/store.ts": () => import("../convex/market/store"),
  "../convex/market/worker.ts": async () => {
    const { internalAction } = await import("../convex/_generated/server")
    const noop = internalAction({ handler: async () => null })
    return { publishPost: noop, publishDelivery: noop, checkStatus: noop, refreshAccount: noop }
  },
  "../convex/_generated/server.js": () => import("../convex/_generated/server"),
}

process.env.MARKET_TOKEN_KEY = btoa(String.fromCharCode(...crypto.getRandomValues(new Uint8Array(32))))

describe("TikTok OAuth form contract", () => {
  let transport: ReturnType<typeof spyOn<typeof globalThis, "fetch">> | undefined
  afterEach(() => transport?.mockRestore())

  test("authorize always shows TikTok's consent page, even for an account that already approved", () => {
    const url = tiktok.authorize({
      client: { id: "id", secret: "secret" },
      redirect: "https://example.com/callback",
      state: "state",
    })
    expect(new URL(url).searchParams.get("disable_auto_auth")).toBe("1")
  })

  for (const grant of ["authorization_code", "refresh_token"] as const) {
    test(`${grant} uses TikTok's exact media type and preserves encoded form values`, async () => {
      const client = { id: "test-client", secret: "test-secret+&=" }
      const redirect = "https://www.cryzo.me/api/market/oauth/tiktok/callback"
      const credential = "test-credential+&=%/"
      transport = spyOn(globalThis, "fetch").mockImplementation(
        Object.assign(
          async (url: RequestInfo | URL, init?: RequestInit) => {
            const request = new Request(url, init)
            expect(request.url).toBe("https://open.tiktokapis.com/v2/oauth/token/")
            expect(request.method).toBe("POST")
            // TikTok rejects the automatic URLSearchParams charset suffix as invalid_request.
            expect(request.headers.get("Content-Type")).toBe("application/x-www-form-urlencoded")
            const body = new URLSearchParams(await request.text())
            expect(body.get("client_key")).toBe(client.id)
            expect(body.get("client_secret")).toBe(client.secret)
            expect(body.get("grant_type")).toBe(grant)
            expect(body.get(grant === "authorization_code" ? "code" : "refresh_token")).toBe(credential)
            if (grant === "authorization_code") expect(body.get("redirect_uri")).toBe(redirect)
            return Response.json({
              access_token: "test-access-token",
              refresh_token: "test-rotated-refresh",
              expires_in: 7200,
              scope: "user.info.basic,user.info.stats",
            })
          },
          { preconnect: fetch.preconnect },
        ),
      )
      const result =
        grant === "authorization_code"
          ? await tiktok.exchange({ client, redirect, code: credential })
          : await tiktok.refresh!({ access: "test-old-access", refresh: credential }, client)
      expect(result.access).toBe("test-access-token")
      expect(result.refresh).toBe("test-rotated-refresh")
      expect(result.scope).toBe("user.info.basic,user.info.stats")
    })
  }
})

describe("Instagram Unified Login", () => {
  let transport: ReturnType<typeof spyOn<typeof globalThis, "fetch">> | undefined
  afterEach(() => {
    transport?.mockRestore()
    delete process.env.META_INSTAGRAM_CONFIG_ID
  })
  const client = { id: "meta-app", secret: "meta-secret" }
  const redirect = "https://www.cryzo.me/api/market/oauth/instagram/callback"

  test("authorize opens Facebook Login for Business with the Instagram configuration, not scopes", () => {
    process.env.META_INSTAGRAM_CONFIG_ID = "123"
    const url = new URL(instagram.authorize({ client, redirect, state: "state" }))
    expect(url.origin + url.pathname).toBe("https://www.facebook.com/v25.0/dialog/oauth")
    expect(url.searchParams.get("config_id")).toBe("123")
    expect(url.searchParams.get("scope")).toBeNull()
    expect(url.searchParams.get("prompt")).toBe("login")
    expect(url.searchParams.get("redirect_uri")).toBe(redirect)
  })

  test("profile picks the only shared Instagram account as the posting destination", async () => {
    transport = spyOn(globalThis, "fetch").mockImplementation(
      Object.assign(
        async (url: RequestInfo | URL) => {
          const path = new URL(new Request(url).url).pathname
          if (path.endsWith("/me/assigned_instagram_accounts")) return Response.json({ data: [{ id: "1784" }] })
          return Response.json({ username: "cryzo.me", profile_picture_url: "https://example.com/a.png" })
        },
        { preconnect: fetch.preconnect },
      ),
    )
    const profile = await instagram.profile({ access: "suat" })
    expect(profile.id).toBe("1784")
    expect(profile.target).toEqual({ id: "1784", name: "@cryzo.me" })
    expect(profile.avatar).toBe("https://example.com/a.png")
  })
})

const facebook = { channel: "facebook", connectedAccountId: "facebook:1", targetId: "123456" }

test.each(["direct", "draft"] as const)("TikTok %s requires a video, not a caption", (mode) => {
  const post = {
    content: "",
    targets: [{ channel: "tiktok" as const, connectedAccountId: "tiktok:1" }],
    options: { tiktokMode: mode, tiktokPrivacy: "SELF_ONLY" },
    mediaTypes: [] as string[],
  }
  const accounts = [{ channel: "tiktok", connectedAccountId: "tiktok:1" }]
  expect(publishProblem(post, accounts)).toBe("TikTok needs exactly one video.")
  expect(publishProblem({ ...post, content: "A caption alone" }, accounts)).toBe("TikTok needs exactly one video.")
  expect(publishProblem({ ...post, mediaTypes: ["video/mp4"] }, accounts)).toBeNull()
  expect(publishProblem({ ...post, mediaTypes: ["image/png"] }, accounts)).toBe("TikTok needs exactly one video.")
  expect(publishProblem({ ...post, mediaTypes: ["video/mp4", "video/mp4"] }, accounts)).toBe(
    "TikTok needs exactly one video.",
  )
})

describe("TikTok video delivery", () => {
  let transport: ReturnType<typeof spyOn<typeof globalThis, "fetch">> | undefined
  afterEach(() => transport?.mockRestore())

  test.each(["direct", "draft"] as const)("%s uploads the attached bytes without requiring a caption", async (mode) => {
    const clip = new Uint8Array([1, 2, 3, 4])
    const fetched: string[] = []
    transport = spyOn(globalThis, "fetch").mockImplementation(
      Object.assign(
        async (input: RequestInfo | URL, init?: RequestInit) => {
          const request = new Request(input, init)
          fetched.push(request.url)
          if (request.url === "https://media.example/clip.mp4") {
            expect(request.headers.get("Range")).toBe("bytes=0-3")
            return new Response(clip, { status: 206 })
          }
          expect(request.url).toBe("https://open.tiktokapis.com/v2/post/publish/creator_info/query/")
          return Response.json({ data: { privacy_level_options: ["SELF_ONLY"] }, error: { code: "ok" } })
        },
        { preconnect: fetch.preconnect },
      ),
    )
    const sent: string[] = []
    const result = await tiktok.publish(
      {
        tokens: { access: "fixture-access" },
        content: "",
        options: { tiktokMode: mode, tiktokPrivacy: "SELF_ONLY" },
        media: [{ url: "https://media.example/clip.mp4", type: "video/mp4", size: clip.length }],
      },
      async (url, init, publishes) => {
        sent.push(url)
        if (url === "https://upload.example/video") {
          expect(init?.method).toBe("PUT")
          expect(new Headers(init?.headers).get("Content-Range")).toBe("bytes 0-3/4")
          expect(new Uint8Array(await new Response(init?.body).arrayBuffer())).toEqual(clip)
          return new Response(null, { status: 204 })
        }
        expect(url).toBe(
          `https://open.tiktokapis.com/v2/post/publish/${mode === "draft" ? "inbox/video" : "video"}/init/`,
        )
        expect(publishes).toBe(true)
        expect(init?.method).toBe("POST")
        const source_info = { source: "FILE_UPLOAD", video_size: 4, chunk_size: 4, total_chunk_count: 1 }
        if (mode === "draft") expect(init?.json).toEqual({ source_info })
        else
          expect(init?.json).toEqual({
            source_info,
            post_info: {
              title: "",
              privacy_level: "SELF_ONLY",
              disable_comment: true,
              disable_duet: true,
              disable_stitch: true,
              brand_content_toggle: false,
              brand_organic_toggle: false,
            },
          })
        return Response.json({
          data: { publish_id: "fixture-publish", upload_url: "https://upload.example/video" },
          error: { code: "ok" },
        })
      },
    )
    expect(result).toEqual({ id: "fixture-publish", pending: true })
    expect(sent).toHaveLength(2)
    expect(fetched).toHaveLength(mode === "draft" ? 1 : 2)
  })
})

test("publishing rules name the first problem per network", () => {
  const post = {
    content: "Launch day",
    targets: [{ channel: "facebook" as const, connectedAccountId: "facebook:1" }],
    mediaTypes: [],
  }
  expect(publishProblem(post, [])).toBe("Connect and select facebook before publishing.")
  expect(publishProblem(post, [{ ...facebook, targetId: undefined }])).toBe("Choose the Facebook Page to post to.")
  expect(publishProblem(post, [{ ...facebook, needsReconnect: true }])).toBe("Reconnect Facebook before publishing.")
  expect(publishProblem(post, [facebook])).toBeNull()
  const reddit = {
    content: "Hi",
    targets: [{ channel: "reddit" as const, connectedAccountId: "reddit:1" }],
    mediaTypes: [],
  }
  expect(publishProblem(reddit, [{ channel: "reddit", connectedAccountId: "reddit:1" }])).toBe(
    "Choose a Reddit community.",
  )
  const tiktok = {
    content: "Hi",
    targets: [{ channel: "tiktok" as const, connectedAccountId: "tiktok:1" }],
    mediaTypes: ["image/png"],
  }
  expect(publishProblem(tiktok, [{ channel: "tiktok", connectedAccountId: "tiktok:1" }])).toBe(
    "TikTok needs exactly one video.",
  )
  const clip = { ...tiktok, mediaTypes: ["video/mp4"] }
  const account = [{ channel: "tiktok", connectedAccountId: "tiktok:1" }]
  expect(publishProblem(clip, account)).toBe("Choose who can see the TikTok video.")
  expect(publishProblem({ ...clip, options: { tiktokPrivacy: "SELF_ONLY", tiktokDisclose: true } }, account)).toBe(
    "Choose whether the TikTok video promotes your brand, a third party, or both.",
  )
  expect(
    publishProblem(
      { ...clip, options: { tiktokPrivacy: "SELF_ONLY", tiktokDisclose: true, tiktokBranded: true } },
      account,
    ),
  ).toBe("Branded content on TikTok can't be private.")
  expect(publishProblem({ ...clip, options: { tiktokPrivacy: "SELF_ONLY" } }, account)).toBeNull()
  // Visibility and disclosure are chosen in the TikTok app for a draft.
  expect(publishProblem({ ...clip, options: { tiktokMode: "draft" as const } }, account)).toBeNull()
  const x = {
    content: "x".repeat(281),
    targets: [{ channel: "x" as const, connectedAccountId: "x:1" }],
    mediaTypes: [],
  }
  expect(publishProblem(x, [{ channel: "x", connectedAccountId: "x:1" }])).toBe(
    "X posts must be 280 characters or fewer.",
  )
  expect(
    publishProblem({ ...x, content: "clip", mediaTypes: ["video/mp4"] }, [{ channel: "x", connectedAccountId: "x:1" }]),
  ).toBeNull()
  const linkedin = {
    content: "Hi",
    targets: [{ channel: "linkedin" as const, connectedAccountId: "linkedin:1" }],
    mediaTypes: [],
  }
  expect(publishProblem(linkedin, [{ channel: "linkedin", connectedAccountId: "linkedin:1" }])).toBe(
    "Choose the LinkedIn profile or page to post to.",
  )
})

test("graph insights keep daily series and report the latest value for follower counts", () => {
  const result = graphMetrics({
    data: {
      data: [
        {
          name: "page_follows",
          values: [
            { value: 10, end_time: "2026-10-01T07:00:00" },
            { value: 12, end_time: "2026-10-02T07:00:00" },
          ],
        },
        { name: "page_post_engagements", values: [{ value: 3 }, { value: 4 }] },
      ],
    },
  })
  expect(result.metrics).toEqual([
    { key: "page_follows", value: 12, window: "snapshot" },
    { key: "page_post_engagements", value: 7, window: "period" },
  ])
  expect(result.series[0].points[1]).toEqual({ date: "2026-10-02", value: 12 })
})

test("tokens are sealed with a fresh nonce and the PKCE challenge is base64url SHA-256", async () => {
  const tokens = { access: "secret-access", refresh: "secret-refresh" }
  const sealed = await seal(tokens)
  expect(sealed).not.toContain("secret")
  expect(await seal(tokens)).not.toBe(sealed)
  expect(await open(sealed)).toEqual(tokens)
  await expect(open(sealed.replace(/.$/, (char) => (char === "A" ? "B" : "A")))).rejects.toThrow()
  expect(await challenge("dBjftJeZ4CVP-mJ92K9qkjM8iZ0iA1DRE6r2_t4BqGk")).toBe(
    "7EEvpBCx-1Tyz5iTPX6mFwr9nKIJQLGK7Au-w3vrDjU",
  )
})

test("a sign-in state is single use and bound to its network", async () => {
  const t = convexTest(schema, modules)
  await t.mutation(internal.market.store.startOauth, { user: "alice", channel: "x", state: "s1", verifier: "v1" })
  expect(await t.mutation(internal.market.store.takeOauth, { state: "s1", channel: "x" })).toEqual({
    user: "alice",
    verifier: "v1",
  })
  expect(await t.mutation(internal.market.store.takeOauth, { state: "s1", channel: "x" })).toBeNull()
  await t.mutation(internal.market.store.startOauth, { user: "alice", channel: "x", state: "s2" })
  expect(await t.mutation(internal.market.store.takeOauth, { state: "s2", channel: "tiktok" })).toBeNull()
  expect(await t.mutation(internal.market.store.takeOauth, { state: "s2", channel: "x" })).toBeNull()
  await t.run(async (ctx) =>
    ctx.db.insert("marketOauth", { state: "s3", user: "alice", channel: "x", expiresAt: Date.now() - 1 }),
  )
  expect(await t.mutation(internal.market.store.takeOauth, { state: "s3", channel: "x" })).toBeNull()
})

test("reconnecting replaces tokens without duplicating the channel, and clients never see tokens", async () => {
  const t = convexTest(schema, modules)
  const connect = async (token: string) =>
    t.mutation(internal.market.store.upsertAccount, {
      user: "alice",
      channel: "facebook",
      providerUserId: "42",
      name: "Alice",
      token: await seal({ access: token }),
      expiresAt: Date.now() + 60 * 86_400_000,
      refreshable: false,
    })
  expect(await connect("first")).toBe("facebook:42")
  await t.mutation(internal.market.store.updateAccount, {
    user: "alice",
    connectedAccountId: "facebook:42",
    name: "Brand",
  })
  await t.mutation(internal.market.store.setTarget, {
    user: "alice",
    connectedAccountId: "facebook:42",
    targetId: "99",
    targetName: "Page",
    targetToken: "sealed",
  })
  const id = (await t.query(internal.market.store.account, { user: "alice", connectedAccountId: "facebook:42" }))!._id
  await t.mutation(internal.market.store.flagReconnect, { accountId: id })
  await connect("second")
  const { accounts } = await t.query(internal.market.store.listAccounts, { user: "alice" })
  expect(accounts).toHaveLength(1)
  expect(accounts[0]).toMatchObject({ name: "Brand", targetId: "99", needsReconnect: false, needsTarget: false })
  expect(JSON.stringify(accounts)).not.toContain("sealed")
  expect(Object.keys(accounts[0])).not.toContain("token")
  const row = await t.query(internal.market.store.account, { user: "alice", connectedAccountId: "facebook:42" })
  expect(await open(row!.token)).toEqual({ access: "second" })
  expect((await t.query(internal.market.store.listAccounts, { user: "mallory" })).accounts).toHaveLength(0)
})

async function setup() {
  const t = convexTest(schema, modules)
  await t.run(async (ctx) => {
    await ctx.db.insert("marketAccounts", {
      user: "alice",
      channel: "facebook",
      connectedAccountId: "facebook:1",
      providerUserId: "1",
      targetId: "123456",
      name: "Alice Page",
      token: "sealed",
      refreshable: false,
      createdAt: Date.now(),
    })
  })
  return t
}

const create = (status: "draft" | "scheduled", scheduledFor = Date.now() + 86_400_000, requestKey = "req_1") => ({
  user: "alice",
  requestKey,
  content: "Launch day",
  targets: [{ channel: "facebook" as const, connectedAccountId: "facebook:1" }],
  scheduledFor,
  status,
  mediaIds: [],
})

test("a retried create returns the same post", async () => {
  const t = await setup()
  const first = await t.mutation(internal.market.store.createPost, create("draft"))
  expect(await t.mutation(internal.market.store.createPost, create("draft"))).toBe(first)
})

test("scheduling validates the channel and other users cannot see the post", async () => {
  const t = await setup()
  await expect(
    t.mutation(internal.market.store.createPost, {
      ...create("scheduled"),
      targets: [{ channel: "x", connectedAccountId: "x:missing" }],
    }),
  ).rejects.toThrow("Connect and select x")
  await t.mutation(internal.market.store.createPost, create("scheduled"))
  const range = { from: Date.now(), to: Date.now() + 7 * 86_400_000 }
  expect(await t.query(internal.market.store.listPosts, { user: "alice", ...range })).toHaveLength(1)
  expect(await t.query(internal.market.store.listPosts, { user: "mallory", ...range })).toHaveLength(0)
})

test("a channel that needs reconnecting cannot be scheduled", async () => {
  const t = await setup()
  const row = await t.query(internal.market.store.account, { user: "alice", connectedAccountId: "facebook:1" })
  await t.mutation(internal.market.store.flagReconnect, { accountId: row!._id })
  await expect(t.mutation(internal.market.store.createPost, create("scheduled"))).rejects.toThrow("Reconnect Facebook")
})

test("a dispatch with an unknown outcome is never published again automatically", async () => {
  const t = await setup()
  const postId = await t.mutation(internal.market.store.createPost, create("scheduled"))
  // Not due yet: the scheduled run exits without claiming.
  expect(await t.mutation(internal.market.store.claimPost, { postId })).toEqual([])
  await t.run(async (ctx) => ctx.db.patch(postId, { scheduledFor: Date.now() - 1000 }))
  const [deliveryId] = await t.mutation(internal.market.store.claimPost, { postId })
  expect(await t.mutation(internal.market.store.claimDelivery, { deliveryId })).toBe(true)
  // A second worker cannot claim the same delivery.
  expect(await t.mutation(internal.market.store.claimDelivery, { deliveryId })).toBe(false)
  await t.mutation(internal.market.store.markFailed, { deliveryId, error: "timeout", outcomeUnknown: true })
  expect((await t.run(async (ctx) => ctx.db.get(postId)))?.status).toBe("failed")
  await expect(t.mutation(internal.market.store.publishNow, { user: "alice", postId })).rejects.toThrow(
    "result is unknown",
  )
  await expect(t.mutation(internal.market.store.retryDelivery, { user: "alice", deliveryId })).rejects.toThrow(
    "Check the network",
  )
  await t.finishInProgressScheduledFunctions()
})

test("publishing all deliveries marks the post published", async () => {
  const t = await setup()
  const postId = await t.mutation(internal.market.store.createPost, create("scheduled"))
  await t.run(async (ctx) => ctx.db.patch(postId, { scheduledFor: Date.now() - 1000 }))
  const [deliveryId] = await t.mutation(internal.market.store.claimPost, { postId })
  await t.mutation(internal.market.store.claimDelivery, { deliveryId })
  await t.mutation(internal.market.store.markPublished, {
    deliveryId,
    operation: "/v25.0/1/feed",
    remotePostId: "123_456",
  })
  expect((await t.run(async (ctx) => ctx.db.get(postId)))?.status).toBe("published")
  await t.finishInProgressScheduledFunctions()
  await expect(
    t.mutation(internal.market.store.updatePost, { user: "alice", postId, content: "edit" }),
  ).rejects.toThrow("cannot be edited")
  await t.finishInProgressScheduledFunctions()
})

test("reconnecting or selecting a different destination invalidates cached analytics", async () => {
  const t = await setup()
  const key = { user: "alice", connectedAccountId: "facebook:1", days: 30 }
  await t.mutation(internal.market.store.saveAnalytics, { ...key, data: { metrics: [{ key: "followers", value: 5 }] } })
  await t.mutation(internal.market.store.setTarget, {
    user: "alice",
    connectedAccountId: "facebook:1",
    targetId: "new-page",
    targetName: "New Page",
  })
  expect(await t.query(internal.market.store.analytics, key)).toBeNull()
  await t.mutation(internal.market.store.saveAnalytics, { ...key, data: { metrics: [] } })
  await t.mutation(internal.market.store.upsertAccount, {
    user: "alice",
    channel: "facebook",
    providerUserId: "1",
    name: "Alice",
    token: "sealed-replacement",
    refreshable: false,
  })
  expect(await t.query(internal.market.store.analytics, key)).toBeNull()
})

describe("LinkedIn analytics", () => {
  const originalEnabled = process.env.LINKEDIN_MEMBER_ANALYTICS_ENABLED
  const originalPending = process.env.MARKET_PENDING
  let transport: ReturnType<typeof spyOn<typeof globalThis, "fetch">> | undefined
  afterEach(() => {
    transport?.mockRestore()
    transport = undefined
    if (originalEnabled === undefined) delete process.env.LINKEDIN_MEMBER_ANALYTICS_ENABLED
    else process.env.LINKEDIN_MEMBER_ANALYTICS_ENABLED = originalEnabled
    if (originalPending === undefined) delete process.env.MARKET_PENDING
    else process.env.MARKET_PENDING = originalPending
  })
  const input = () => ({
    tokens: { access: "test-token", scope: "r_member_postAnalytics" },
    targetId: "urn:li:person:123",
    days: 30,
    from: new Date("2026-10-04T00:00:00Z"),
    to: new Date("2026-11-02T19:00:00Z"),
    published: [],
  })

  test("approval and consent gates do not invent metrics or request unapproved scopes", async () => {
    delete process.env.LINKEDIN_MEMBER_ANALYTICS_ENABLED
    expect((await linkedin.metrics(input())).permissionReason).toBe("approval_pending")
    expect(
      new URL(
        linkedin.authorize({
          client: { id: "id", secret: "secret" },
          redirect: "https://example.com/callback",
          state: "state",
        }),
      ).searchParams.get("scope"),
    ).not.toContain("r_member_postAnalytics")
    process.env.LINKEDIN_MEMBER_ANALYTICS_ENABLED = "true"
    expect(
      (await linkedin.metrics({ ...input(), tokens: { access: "test", scope: "openid profile w_member_social" } }))
        .permissionReason,
    ).toBe("reconnect_required")
  })

  test("member reports use bounded dates, omit denied metrics and preserve available totals", async () => {
    process.env.LINKEDIN_MEMBER_ANALYTICS_ENABLED = "true"
    const requests: URL[] = []
    transport = spyOn(globalThis, "fetch").mockImplementation(
      Object.assign(
        async (url: RequestInfo | URL, init?: RequestInit) => {
          const request = new URL(String(url))
          requests.push(request)
          expect(new Headers(init?.headers).get("LinkedIn-Version")).toBe("202610")
          if (request.searchParams.get("queryType") === "COMMENT") return new Response("denied", { status: 403 })
          return Response.json({
            elements:
              request.searchParams.get("aggregation") === "TOTAL"
                ? [{ count: 11 }]
                : [{ count: 3, dateRange: { start: { year: 2026, month: 11, day: 1 } } }],
          })
        },
        { preconnect: fetch.preconnect },
      ),
    )
    const report = await linkedin.metrics(input())
    expect(report.status).toBe("partial")
    expect(report.metrics).toHaveLength(4)
    expect(report.metrics.find((metric) => metric.key === "comment")).toBeUndefined()
    expect(report.metrics[0]).toMatchObject({ value: 11, window: "period" })
    expect(report.series[0].points[0]).toEqual({ date: "2026-11-01", value: 3 })
    expect(
      requests.every(
        (url) =>
          url.searchParams.get("dateRange") === "(start:(year:2026,month:10,day:4),end:(year:2026,month:11,day:3))",
      ),
    ).toBe(true)
    expect(
      requests.some(
        (url) =>
          url.searchParams.get("queryType") === "MEMBERS_REACHED" && url.searchParams.get("aggregation") === "DAILY",
      ),
    ).toBe(false)
  })

  test("Page reports distinguish current followers from selected-period organic totals", async () => {
    process.env.MARKET_PENDING = ""
    transport = spyOn(globalThis, "fetch").mockImplementation(
      Object.assign(
        async (url: RequestInfo | URL) => {
          const request = new URL(String(url))
          if (request.pathname.endsWith("networkSizes/urn%3Ali%3Aorganization%3A456"))
            return Response.json({ firstDegreeSize: 250 })
          expect(request.searchParams.get("timeIntervals")).toContain("timeGranularityType:DAY")
          return Response.json({
            elements: [
              {
                timeRange: { start: Date.parse("2026-11-01T00:00:00Z") },
                totalShareStatistics: { impressionCount: 10, clickCount: 2 },
              },
              {
                timeRange: { start: Date.parse("2026-11-02T00:00:00Z") },
                totalShareStatistics: { impressionCount: 5, clickCount: 1 },
              },
            ],
          })
        },
        { preconnect: fetch.preconnect },
      ),
    )
    const report = await linkedin.metrics({
      ...input(),
      targetId: "urn:li:organization:456",
      tokens: { access: "test", scope: "rw_organization_admin" },
    })
    expect(report.metrics).toContainEqual({ key: "followers_count", value: 250, window: "snapshot" })
    expect(report.metrics).toContainEqual({ key: "impressionCount", value: 15, window: "period" })
    expect(report.metrics.some((metric) => metric.key === "likeCount")).toBe(false)
  })
})
