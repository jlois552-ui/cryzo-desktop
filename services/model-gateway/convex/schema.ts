import { defineSchema, defineTable } from "convex/server"
import { v } from "convex/values"

export const channel = v.union(
  v.literal("x"),
  v.literal("linkedin"),
  v.literal("facebook"),
  v.literal("instagram"),
  v.literal("youtube"),
  v.literal("tiktok"),
  v.literal("reddit"),
)
export const options = v.object({
  redditCommunity: v.optional(v.string()),
  youtubeTitle: v.optional(v.string()),
  youtubePrivacy: v.optional(v.string()),
  tiktokPrivacy: v.optional(v.string()),
  instagramPostType: v.optional(v.union(v.literal("post"), v.literal("reel"), v.literal("story"), v.literal("carousel"))),
})

export default defineSchema({
  usage: defineTable({ user: v.string(), day: v.string(), calls: v.number() }).index("by_user_and_day", ["user", "day"]),
  calls: defineTable({ user: v.string(), step: v.string(), expires: v.number(), status: v.union(v.literal("active"), v.literal("finished")) })
    .index("by_user_and_step", ["user", "step"]).index("by_status_and_expires", ["status", "expires"]),
  pool: defineTable({ key: v.string(), minute: v.number(), calls: v.number(), blockedUntil: v.optional(v.number()) }).index("by_key", ["key"]),

  // Marketing. `user` is the Better Auth user ID.
  marketGroups: defineTable({ user: v.string(), name: v.string(), color: v.string(), createdAt: v.number() }).index("by_user", ["user"]),
  marketAccounts: defineTable({
    user: v.string(),
    channel: channel,
    // `<channel>:<network user ID>`; posts and deliveries refer to accounts by it.
    connectedAccountId: v.string(),
    providerUserId: v.string(),
    name: v.string(),
    avatar: v.optional(v.string()),
    // Sealed OAuth tokens (lib/crypto). Never returned to clients.
    token: v.string(),
    expiresAt: v.optional(v.number()),
    refreshable: v.boolean(),
    needsReconnect: v.optional(v.boolean()),
    // Facebook Page, Instagram account or LinkedIn member/organization that posts and analytics use.
    targetId: v.optional(v.string()),
    targetName: v.optional(v.string()),
    targetToken: v.optional(v.string()),
    groupId: v.optional(v.id("marketGroups")),
    createdAt: v.number(),
  })
    .index("by_user", ["user"])
    .index("by_user_and_connected", ["user", "connectedAccountId"]),
  marketPosts: defineTable({
    user: v.string(),
    requestKey: v.string(),
    groupId: v.optional(v.id("marketGroups")),
    content: v.string(),
    targets: v.array(v.object({ channel: channel, connectedAccountId: v.string(), content: v.optional(v.string()) })),
    scheduledFor: v.number(),
    status: v.union(v.literal("draft"), v.literal("scheduled"), v.literal("publishing"), v.literal("published"), v.literal("failed")),
    mediaIds: v.array(v.id("_storage")),
    options: v.optional(options),
    error: v.optional(v.string()),
    publishedAt: v.optional(v.number()),
    createdAt: v.number(),
    updatedAt: v.number(),
  })
    .index("by_user_and_scheduled", ["user", "scheduledFor"])
    .index("by_user_and_request", ["user", "requestKey"]),
  marketDeliveries: defineTable({
    user: v.string(),
    postId: v.id("marketPosts"),
    channel: channel,
    connectedAccountId: v.string(),
    status: v.union(v.literal("pending"), v.literal("publishing"), v.literal("published"), v.literal("failed"), v.literal("unknown")),
    attempts: v.number(),
    leaseExpiresAt: v.optional(v.number()),
    pollingAttempts: v.optional(v.number()),
    operation: v.optional(v.string()),
    remotePostId: v.optional(v.string()),
    remoteUrl: v.optional(v.string()),
    error: v.optional(v.string()),
    updatedAt: v.number(),
  }).index("by_post", ["postId"]),
  // Pending OAuth sign-ins: single use, expire after 10 minutes.
  marketOauth: defineTable({ state: v.string(), user: v.string(), channel: channel, verifier: v.optional(v.string()), expiresAt: v.number() }).index(
    "by_state",
    ["state"],
  ),
  marketMedia: defineTable({ user: v.string(), storageId: v.id("_storage"), contentType: v.string(), name: v.string(), size: v.number(), createdAt: v.number() })
    .index("by_user", ["user"])
    .index("by_storage", ["storageId"]),
  marketAnalytics: defineTable({ user: v.string(), connectedAccountId: v.string(), days: v.number(), data: v.any(), updatedAt: v.number() })
    .index("by_user_account_days", ["user", "connectedAccountId", "days"]),
})
