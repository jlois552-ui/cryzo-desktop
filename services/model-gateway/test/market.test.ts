import { linkedin } from "../convex/lib/providers/linkedin"
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

const facebook = { channel: "facebook", connectedAccountId: "facebook:1", targetId: "123456" }

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
