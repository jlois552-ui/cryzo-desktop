import { expect, test } from "bun:test"
import { convexTest } from "convex-test"
import schema from "../convex/schema"
import { internal } from "../convex/_generated/api"
import { graphMetrics, publishProblem } from "../convex/lib/market"

// The Composio worker is replaced by no-ops: scheduled publishing runs must not reach the network.
const modules = {
  "../convex/market/store.ts": () => import("../convex/market/store"),
  "../convex/market/worker.ts": async () => {
    const { internalAction } = await import("../convex/_generated/server")
    const noop = internalAction({ handler: async () => null })
    return { publishPost: noop, publishDelivery: noop, checkTikTok: noop }
  },
  "../convex/_generated/server.js": () => import("../convex/_generated/server"),
}

const facebook = { channel: "facebook", connectedAccountId: "ca_fb", targetId: "123456" }

test("publishing rules name the first problem per network", () => {
  const post = { content: "Launch day", targets: [{ channel: "facebook" as const, connectedAccountId: "ca_fb" }], mediaTypes: [] }
  expect(publishProblem(post, [])).toBe("Connect and select facebook before publishing.")
  expect(publishProblem(post, [{ ...facebook, targetId: undefined }])).toBe("Choose the Facebook Page to post to.")
  expect(publishProblem(post, [facebook])).toBeNull()
  const reddit = { content: "Hi", targets: [{ channel: "reddit" as const, connectedAccountId: "ca_r" }], mediaTypes: [] }
  expect(publishProblem(reddit, [{ channel: "reddit", connectedAccountId: "ca_r" }])).toBe("Choose a Reddit community.")
  const tiktok = { content: "Hi", targets: [{ channel: "tiktok" as const, connectedAccountId: "ca_t" }], mediaTypes: ["image/png"] }
  expect(publishProblem(tiktok, [{ channel: "tiktok", connectedAccountId: "ca_t" }])).toBe("TikTok needs exactly one video.")
  const x = { content: "x".repeat(281), targets: [{ channel: "x" as const, connectedAccountId: "ca_x" }], mediaTypes: [] }
  expect(publishProblem(x, [{ channel: "x", connectedAccountId: "ca_x" }])).toBe("X posts must be 280 characters or fewer.")
})

test("graph insights keep daily series and report the latest value for follower counts", () => {
  const result = graphMetrics({
    data: {
      data: [
        { name: "page_follows", values: [{ value: 10, end_time: "2026-10-01T07:00:00" }, { value: 12, end_time: "2026-10-02T07:00:00" }] },
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

async function setup() {
  const t = convexTest(schema, modules)
  await t.run(async (ctx) => {
    await ctx.db.insert("marketAccounts", { user: "alice", ...facebook, channel: "facebook", name: "Alice Page", createdAt: Date.now() })
  })
  return t
}

const create = (status: "draft" | "scheduled", scheduledFor = Date.now() + 86_400_000, requestKey = "req_1") => ({
  user: "alice",
  requestKey,
  content: "Launch day",
  targets: [{ channel: "facebook" as const, connectedAccountId: "ca_fb" }],
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
    t.mutation(internal.market.store.createPost, { ...create("scheduled"), targets: [{ channel: "x", connectedAccountId: "ca_missing" }] }),
  ).rejects.toThrow("Connect and select x")
  await t.mutation(internal.market.store.createPost, create("scheduled"))
  const range = { from: Date.now(), to: Date.now() + 7 * 86_400_000 }
  expect(await t.query(internal.market.store.listPosts, { user: "alice", ...range })).toHaveLength(1)
  expect(await t.query(internal.market.store.listPosts, { user: "mallory", ...range })).toHaveLength(0)
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
  const post = await t.run(async (ctx) => ctx.db.get(postId))
  expect(post?.status).toBe("failed")
  await expect(t.mutation(internal.market.store.publishNow, { user: "alice", postId })).rejects.toThrow("result is unknown")
  await expect(t.mutation(internal.market.store.retryDelivery, { user: "alice", deliveryId })).rejects.toThrow("Check the network")
  await t.finishInProgressScheduledFunctions()
})

test("publishing all deliveries marks the post published", async () => {
  const t = await setup()
  const postId = await t.mutation(internal.market.store.createPost, create("scheduled"))
  await t.run(async (ctx) => ctx.db.patch(postId, { scheduledFor: Date.now() - 1000 }))
  const [deliveryId] = await t.mutation(internal.market.store.claimPost, { postId })
  await t.mutation(internal.market.store.claimDelivery, { deliveryId })
  await t.mutation(internal.market.store.markPublished, { deliveryId, toolSlug: "FACEBOOK_CREATE_POST", remotePostId: "123_456" })
  expect((await t.run(async (ctx) => ctx.db.get(postId)))?.status).toBe("published")
  await expect(t.mutation(internal.market.store.updatePost, { user: "alice", postId, content: "edit" })).rejects.toThrow("cannot be edited")
  await t.finishInProgressScheduledFunctions()
})
