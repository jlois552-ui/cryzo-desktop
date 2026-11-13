// Marketing data. Every function takes the user resolved by the HTTP layer from the
// desktop's Better Auth session; none is callable from clients directly.
import { v } from "convex/values"
import { internal } from "../_generated/api"
import type { Doc, Id } from "../_generated/dataModel"
import { internalMutation, internalQuery, type MutationCtx, type QueryCtx } from "../_generated/server"
import { postStatus, publishProblem, type Channel } from "../lib/market"
import { channel, options } from "../schema"

const target = v.object({ channel, connectedAccountId: v.string(), content: v.optional(v.string()) })
type Ctx = QueryCtx | MutationCtx

async function accounts(ctx: Ctx, user: string) {
  return await ctx.db
    .query("marketAccounts")
    .withIndex("by_user", (q) => q.eq("user", user))
    .take(200)
}

async function owned(ctx: Ctx, user: string, connectedAccountId: string) {
  return await ctx.db
    .query("marketAccounts")
    .withIndex("by_user_and_connected", (q) => q.eq("user", user).eq("connectedAccountId", connectedAccountId))
    .unique()
}

// Networks that post as a chosen Page, business account or member/organization.
const targeted: Channel[] = ["facebook", "instagram", "linkedin"]

/** An account as clients see it: tokens removed, connection state derived. */
function view(row: Doc<"marketAccounts">) {
  const { token, targetToken, ...rest } = row
  const expired = row.expiresAt !== undefined && !row.refreshable && row.expiresAt <= Date.now()
  return {
    ...rest,
    needsReconnect: Boolean(row.needsReconnect || expired),
    needsTarget: targeted.includes(row.channel) && !row.targetId,
  }
}

async function deliveries(ctx: Ctx, postId: Id<"marketPosts">) {
  return await ctx.db
    .query("marketDeliveries")
    .withIndex("by_post", (q) => q.eq("postId", postId))
    .take(20)
}

async function ownedPost(ctx: Ctx, user: string, postId: Id<"marketPosts">) {
  const post = await ctx.db.get(postId)
  if (!post || post.user !== user) throw new Error("Post not found.")
  return post
}

async function mediaTypes(ctx: Ctx, user: string, ids: Id<"_storage">[]) {
  return await Promise.all(
    ids.map(async (storageId) => {
      const media = await ctx.db
        .query("marketMedia")
        .withIndex("by_storage", (q) => q.eq("storageId", storageId))
        .unique()
      if (!media || media.user !== user) throw new Error("Media does not belong to this account.")
      return media.contentType
    }),
  )
}

async function problem(
  ctx: Ctx,
  post: Pick<Doc<"marketPosts">, "user" | "content" | "targets" | "options" | "mediaIds">,
) {
  return publishProblem(
    {
      content: post.content,
      targets: post.targets,
      options: post.options,
      mediaTypes: await mediaTypes(ctx, post.user, post.mediaIds),
    },
    (await accounts(ctx, post.user)).map(view),
  )
}

async function refresh(ctx: MutationCtx, postId: Id<"marketPosts">) {
  const items = await deliveries(ctx, postId)
  if (!items.length) return
  const status = postStatus(items)
  const unknown = items.filter((item) => item.status === "unknown").length
  const failed = items.filter((item) => item.status === "failed")
  await ctx.db.patch(postId, {
    status,
    updatedAt: Date.now(),
    ...(status === "published" ? { publishedAt: Date.now(), error: undefined } : {}),
    ...(status === "failed"
      ? {
          error: unknown
            ? "A network's result is unknown. Check it before retrying to avoid a duplicate post."
            : (failed[0]?.error ?? "Publishing failed."),
        }
      : {}),
  })
}

// Accounts and groups

export const listAccounts = internalQuery({
  args: { user: v.string() },
  handler: async (ctx, args) => ({
    accounts: (await accounts(ctx, args.user)).map(view),
    groups: await ctx.db
      .query("marketGroups")
      .withIndex("by_user", (q) => q.eq("user", args.user))
      .take(100),
  }),
})

/** The full account row, tokens included, for the worker. */
export const account = internalQuery({
  args: { user: v.string(), connectedAccountId: v.string() },
  handler: async (ctx, args) => await owned(ctx, args.user, args.connectedAccountId),
})

export const accountById = internalQuery({
  args: { accountId: v.id("marketAccounts") },
  handler: async (ctx, args) => await ctx.db.get(args.accountId),
})

/** Adds a connected account, or replaces the tokens of one connected before (reconnect). */
export const upsertAccount = internalMutation({
  returns: v.string(),
  args: {
    user: v.string(),
    channel,
    providerUserId: v.string(),
    name: v.string(),
    avatar: v.optional(v.string()),
    token: v.string(),
    expiresAt: v.optional(v.number()),
    refreshable: v.boolean(),
    targetId: v.optional(v.string()),
    targetName: v.optional(v.string()),
    targetToken: v.optional(v.string()),
  },
  handler: async (ctx, args) => {
    const connectedAccountId = `${args.channel}:${args.providerUserId}`
    const row = await owned(ctx, args.user, connectedAccountId)
    const tokens = { avatar: args.avatar, token: args.token, expiresAt: args.expiresAt, refreshable: args.refreshable }
    // A reconnect keeps the user's name, group and chosen target unless a new target was resolved.
    const target = args.targetId
      ? { targetId: args.targetId, targetName: args.targetName, targetToken: args.targetToken }
      : {}
    const accountId =
      row?._id ??
      (await ctx.db.insert("marketAccounts", {
        user: args.user,
        channel: args.channel,
        connectedAccountId,
        providerUserId: args.providerUserId,
        name: args.name.slice(0, 100),
        ...tokens,
        ...target,
        createdAt: Date.now(),
      }))
    if (row) await ctx.db.patch(row._id, { ...tokens, ...target, needsReconnect: false })
    await clearAnalytics(ctx, args.user, connectedAccountId)
    await schedule(ctx, accountId, args.expiresAt, args.refreshable)
    return connectedAccountId
  },
})

export const saveTokens = internalMutation({
  args: { accountId: v.id("marketAccounts"), token: v.string(), expiresAt: v.optional(v.number()) },
  handler: async (ctx, args) => {
    const row = await ctx.db.get(args.accountId)
    if (!row) return null
    await ctx.db.patch(row._id, { token: args.token, expiresAt: args.expiresAt, needsReconnect: false })
    await schedule(ctx, row._id, args.expiresAt, row.refreshable)
    return null
  },
})

export const flagReconnect = internalMutation({
  args: { accountId: v.id("marketAccounts") },
  handler: async (ctx, args) => {
    if (await ctx.db.get(args.accountId)) await ctx.db.patch(args.accountId, { needsReconnect: true })
    return null
  },
})

export const setTarget = internalMutation({
  returns: v.null(),
  args: {
    user: v.string(),
    connectedAccountId: v.string(),
    targetId: v.string(),
    targetName: v.string(),
    targetToken: v.optional(v.string()),
  },
  handler: async (ctx, args) => {
    const row = await owned(ctx, args.user, args.connectedAccountId)
    if (!row) throw new Error("Channel not found.")
    await ctx.db.patch(row._id, {
      targetId: args.targetId,
      targetName: args.targetName.slice(0, 100),
      targetToken: args.targetToken,
    })
    await clearAnalytics(ctx, args.user, args.connectedAccountId)
    return null
  },
})

// Refresh ten minutes before expiry; stale runs see a later expiry and exit.
async function schedule(
  ctx: MutationCtx,
  accountId: Id<"marketAccounts">,
  expiresAt: number | undefined,
  refreshable: boolean,
) {
  if (expiresAt && refreshable)
    await ctx.scheduler.runAt(
      Math.max(expiresAt - 10 * 60_000, Date.now() + 60_000),
      internal.market.worker.refreshAccount,
      { accountId },
    )
}

export const startOauth = internalMutation({
  args: { user: v.string(), channel, state: v.string(), verifier: v.optional(v.string()) },
  handler: async (ctx, args) => {
    const id = await ctx.db.insert("marketOauth", { ...args, expiresAt: Date.now() + 10 * 60_000 })
    await ctx.scheduler.runAfter(10 * 60_000, internal.market.store.dropOauth, { id })
    return null
  },
})

/** Consumes a sign-in state once. A wrong network, an expired state or a replay returns null. */
export const takeOauth = internalMutation({
  args: { state: v.string(), channel },
  handler: async (ctx, args) => {
    const row = await ctx.db
      .query("marketOauth")
      .withIndex("by_state", (q) => q.eq("state", args.state))
      .unique()
    if (!row) return null
    await ctx.db.delete(row._id)
    if (row.channel !== args.channel || row.expiresAt < Date.now()) return null
    return { user: row.user, verifier: row.verifier }
  },
})

export const dropOauth = internalMutation({
  args: { id: v.id("marketOauth") },
  handler: async (ctx, args) => {
    if (await ctx.db.get(args.id)) await ctx.db.delete(args.id)
    return null
  },
})

export const updateAccount = internalMutation({
  args: {
    user: v.string(),
    connectedAccountId: v.string(),
    name: v.optional(v.string()),
    groupId: v.optional(v.union(v.id("marketGroups"), v.null())),
  },
  handler: async (ctx, args) => {
    const row = await owned(ctx, args.user, args.connectedAccountId)
    if (!row) throw new Error("Channel not found.")
    if (args.groupId) {
      const group = await ctx.db.get(args.groupId)
      if (group?.user !== args.user) throw new Error("Group not found.")
    }
    const name = args.name?.trim()
    if (name !== undefined && (!name || name.length > 100)) throw new Error("Use a name between 1 and 100 characters.")
    await ctx.db.patch(row._id, {
      ...(name ? { name } : {}),
      ...(args.groupId !== undefined ? { groupId: args.groupId ?? undefined } : {}),
    })
    return null
  },
})

export const removeAccount = internalMutation({
  args: { user: v.string(), connectedAccountId: v.string() },
  handler: async (ctx, args) => {
    const row = await owned(ctx, args.user, args.connectedAccountId)
    if (row) await ctx.db.delete(row._id)
    return null
  },
})

export const saveGroup = internalMutation({
  args: { user: v.string(), groupId: v.optional(v.id("marketGroups")), name: v.string(), color: v.string() },
  handler: async (ctx, args) => {
    const name = args.name.trim()
    if (!name || name.length > 60) throw new Error("Use a group name between 1 and 60 characters.")
    if (!/^#[0-9a-fA-F]{6}$/.test(args.color)) throw new Error("Choose a valid color.")
    if (!args.groupId)
      return await ctx.db.insert("marketGroups", { user: args.user, name, color: args.color, createdAt: Date.now() })
    const group = await ctx.db.get(args.groupId)
    if (group?.user !== args.user) throw new Error("Group not found.")
    await ctx.db.patch(args.groupId, { name, color: args.color })
    return args.groupId
  },
})

export const deleteGroup = internalMutation({
  args: { user: v.string(), groupId: v.id("marketGroups") },
  handler: async (ctx, args) => {
    const group = await ctx.db.get(args.groupId)
    if (group?.user !== args.user) throw new Error("Group not found.")
    for (const row of await accounts(ctx, args.user))
      if (row.groupId === args.groupId) await ctx.db.patch(row._id, { groupId: undefined })
    await ctx.db.delete(args.groupId)
    return null
  },
})

// Posts

export const listPosts = internalQuery({
  args: { user: v.string(), from: v.number(), to: v.number() },
  handler: async (ctx, args) => {
    if (!(args.to > args.from) || args.to - args.from > 400 * 86_400_000)
      throw new Error("Choose a range of at most 400 days.")
    const posts = await ctx.db
      .query("marketPosts")
      .withIndex("by_user_and_scheduled", (q) =>
        q.eq("user", args.user).gte("scheduledFor", args.from).lt("scheduledFor", args.to),
      )
      .take(1000)
    return await Promise.all(posts.map(async (post) => ({ ...post, deliveries: await deliveries(ctx, post._id) })))
  },
})

export const createPost = internalMutation({
  args: {
    user: v.string(),
    requestKey: v.string(),
    content: v.string(),
    targets: v.array(target),
    scheduledFor: v.number(),
    status: v.union(v.literal("draft"), v.literal("scheduled")),
    mediaIds: v.array(v.id("_storage")),
    options: v.optional(options),
    groupId: v.optional(v.id("marketGroups")),
  },
  handler: async (ctx, args) => {
    if (!args.requestKey || args.requestKey.length > 200) throw new Error("Invalid request.")
    // A retried create returns the original post instead of scheduling a second one.
    const previous = await ctx.db
      .query("marketPosts")
      .withIndex("by_user_and_request", (q) => q.eq("user", args.user).eq("requestKey", args.requestKey))
      .first()
    if (previous) return previous._id
    if (!Number.isFinite(args.scheduledFor)) throw new Error("Choose a valid date.")
    if (args.content.length > 12_000) throw new Error("Posts must be 12,000 characters or fewer.")
    const { user, ...fields } = args
    const post = { user, ...fields, content: args.content.trim() }
    await mediaTypes(ctx, user, post.mediaIds)
    if (post.status === "scheduled") {
      const reason = await problem(ctx, post)
      if (reason) throw new Error(reason)
    }
    const now = Date.now()
    const id = await ctx.db.insert("marketPosts", { ...post, createdAt: now, updatedAt: now })
    if (post.status === "scheduled")
      await ctx.scheduler.runAt(Math.max(post.scheduledFor, now + 1000), internal.market.worker.publishPost, {
        postId: id,
      })
    return id
  },
})

export const updatePost = internalMutation({
  args: {
    user: v.string(),
    postId: v.id("marketPosts"),
    content: v.optional(v.string()),
    targets: v.optional(v.array(target)),
    scheduledFor: v.optional(v.number()),
    status: v.optional(v.union(v.literal("draft"), v.literal("scheduled"))),
    mediaIds: v.optional(v.array(v.id("_storage"))),
    options: v.optional(options),
    groupId: v.optional(v.union(v.id("marketGroups"), v.null())),
  },
  handler: async (ctx, args) => {
    const post = await ownedPost(ctx, args.user, args.postId)
    if (post.status === "publishing" || post.status === "published")
      throw new Error("Published posts cannot be edited.")
    if (args.scheduledFor !== undefined && !Number.isFinite(args.scheduledFor)) throw new Error("Choose a valid date.")
    const next = {
      content: args.content?.trim() ?? post.content,
      targets: args.targets ?? post.targets,
      scheduledFor: args.scheduledFor ?? post.scheduledFor,
      status: args.status ?? (post.status === "failed" ? "scheduled" : post.status),
      mediaIds: args.mediaIds ?? post.mediaIds,
      options: args.options ?? post.options,
      groupId: args.groupId === null ? undefined : (args.groupId ?? post.groupId),
    }
    await mediaTypes(ctx, args.user, next.mediaIds)
    if (next.status === "scheduled") {
      const reason = await problem(ctx, { user: args.user, ...next })
      if (reason) throw new Error(reason)
    }
    // Deliveries are rebuilt at publish time from the current targets.
    for (const item of await deliveries(ctx, post._id)) await ctx.db.delete(item._id)
    await ctx.db.patch(post._id, { ...next, error: undefined, updatedAt: Date.now() })
    // Earlier scheduled runs see the new time and exit without publishing.
    if (next.status === "scheduled")
      await ctx.scheduler.runAt(Math.max(next.scheduledFor, Date.now() + 1000), internal.market.worker.publishPost, {
        postId: post._id,
      })
    return null
  },
})

export const deletePost = internalMutation({
  args: { user: v.string(), postId: v.id("marketPosts") },
  handler: async (ctx, args) => {
    const post = await ownedPost(ctx, args.user, args.postId)
    if (post.status === "publishing") throw new Error("This post is publishing now.")
    for (const item of await deliveries(ctx, post._id)) await ctx.db.delete(item._id)
    await ctx.db.delete(post._id)
    return null
  },
})

export const publishNow = internalMutation({
  args: { user: v.string(), postId: v.id("marketPosts") },
  handler: async (ctx, args) => {
    const post = await ownedPost(ctx, args.user, args.postId)
    if (post.status === "publishing") throw new Error("This post is already publishing.")
    if (post.status === "published") throw new Error("This post has already been published.")
    if ((await deliveries(ctx, post._id)).some((item) => item.status === "unknown"))
      throw new Error("A network's result is unknown. Check it before publishing again to avoid a duplicate.")
    const reason = await problem(ctx, post)
    if (reason) throw new Error(reason)
    await ctx.db.patch(post._id, {
      status: "scheduled",
      scheduledFor: Date.now(),
      error: undefined,
      updatedAt: Date.now(),
    })
    await ctx.scheduler.runAfter(0, internal.market.worker.publishPost, { postId: post._id })
    return null
  },
})

export const retryDelivery = internalMutation({
  args: { user: v.string(), deliveryId: v.id("marketDeliveries") },
  handler: async (ctx, args) => {
    const delivery = await ctx.db.get(args.deliveryId)
    if (!delivery || delivery.user !== args.user) throw new Error("Delivery not found.")
    if (delivery.status !== "failed")
      throw new Error(
        delivery.status === "unknown"
          ? "Check the network before retrying an unknown result."
          : "Only failed deliveries can be retried.",
      )
    const post = await ownedPost(ctx, args.user, delivery.postId)
    const reason = await problem(ctx, post)
    if (reason) throw new Error(reason)
    await ctx.db.patch(delivery._id, {
      status: "pending",
      error: undefined,
      leaseExpiresAt: undefined,
      updatedAt: Date.now(),
    })
    await ctx.db.patch(post._id, { status: "publishing", error: undefined, updatedAt: Date.now() })
    await ctx.scheduler.runAfter(0, internal.market.worker.publishDelivery, { deliveryId: delivery._id })
    return null
  },
})

// Publishing state machine (worker only)

export const claimPost = internalMutation({
  args: { postId: v.id("marketPosts") },
  handler: async (ctx, args) => {
    const post = await ctx.db.get(args.postId)
    // Stale runs from an earlier schedule exit here.
    if (!post || post.status !== "scheduled" || post.scheduledFor > Date.now()) return []
    const reason = await problem(ctx, post)
    if (reason) {
      await ctx.db.patch(post._id, { status: "failed", error: reason, updatedAt: Date.now() })
      return []
    }
    const existing = await deliveries(ctx, post._id)
    const pending: Id<"marketDeliveries">[] = []
    for (const item of post.targets) {
      const found = existing.find(
        (row) => row.channel === item.channel && row.connectedAccountId === item.connectedAccountId,
      )
      if (found?.status === "published") continue
      if (found) {
        await ctx.db.patch(found._id, { status: "pending", error: undefined, updatedAt: Date.now() })
        pending.push(found._id)
        continue
      }
      pending.push(
        await ctx.db.insert("marketDeliveries", {
          user: post.user,
          postId: post._id,
          channel: item.channel,
          connectedAccountId: item.connectedAccountId,
          status: "pending",
          attempts: 0,
          updatedAt: Date.now(),
        }),
      )
    }
    await ctx.db.patch(post._id, { status: "publishing", error: undefined, updatedAt: Date.now() })
    for (const deliveryId of pending)
      await ctx.scheduler.runAfter(0, internal.market.worker.publishDelivery, { deliveryId })
    return pending
  },
})

export const deliveryForPublish = internalQuery({
  args: { deliveryId: v.id("marketDeliveries") },
  handler: async (ctx, args) => {
    const delivery = await ctx.db.get(args.deliveryId)
    if (!delivery || delivery.status !== "pending") return null
    const post = await ctx.db.get(delivery.postId)
    if (!post || post.status !== "publishing") return null
    const account = await owned(ctx, post.user, delivery.connectedAccountId)
    const media = await Promise.all(
      post.mediaIds.map(async (storageId) => {
        const row = await ctx.db
          .query("marketMedia")
          .withIndex("by_storage", (q) => q.eq("storageId", storageId))
          .unique()
        return { url: (await ctx.storage.getUrl(storageId)) ?? "", type: row?.contentType ?? "", size: row?.size ?? 0 }
      }),
    )
    const content = post.targets.find(
      (item) => item.channel === delivery.channel && item.connectedAccountId === delivery.connectedAccountId,
    )?.content
    return { delivery, post, account, content: (content ?? post.content).trim(), media }
  },
})

export const claimDelivery = internalMutation({
  args: { deliveryId: v.id("marketDeliveries") },
  handler: async (ctx, args) => {
    const delivery = await ctx.db.get(args.deliveryId)
    if (!delivery || delivery.status !== "pending") return false
    await ctx.db.patch(delivery._id, {
      status: "publishing",
      attempts: delivery.attempts + 1,
      leaseExpiresAt: Date.now() + 300_000,
      error: undefined,
      updatedAt: Date.now(),
    })
    await ctx.scheduler.runAfter(305_000, internal.market.store.expireLease, { deliveryId: delivery._id })
    return true
  },
})

export const markPublished = internalMutation({
  args: {
    deliveryId: v.id("marketDeliveries"),
    operation: v.string(),
    remotePostId: v.optional(v.string()),
    remoteUrl: v.optional(v.string()),
  },
  handler: async (ctx, args) => {
    const { deliveryId, ...fields } = args
    const delivery = await ctx.db.get(deliveryId)
    if (!delivery) return null
    await ctx.db.patch(deliveryId, {
      ...fields,
      status: "published",
      error: undefined,
      leaseExpiresAt: undefined,
      updatedAt: Date.now(),
    })
    await refresh(ctx, delivery.postId)
    return null
  },
})

export const markFailed = internalMutation({
  args: {
    deliveryId: v.id("marketDeliveries"),
    error: v.string(),
    outcomeUnknown: v.boolean(),
    operation: v.optional(v.string()),
  },
  handler: async (ctx, args) => {
    const delivery = await ctx.db.get(args.deliveryId)
    if (!delivery) return null
    await ctx.db.patch(delivery._id, {
      status: args.outcomeUnknown ? "unknown" : "failed",
      error: args.error.slice(0, 1000),
      operation: args.operation,
      leaseExpiresAt: undefined,
      updatedAt: Date.now(),
    })
    await refresh(ctx, delivery.postId)
    return null
  },
})

export const markProcessing = internalMutation({
  args: { deliveryId: v.id("marketDeliveries"), remotePostId: v.string(), operation: v.string() },
  handler: async (ctx, args) => {
    const { deliveryId, ...fields } = args
    await ctx.db.patch(deliveryId, { ...fields, status: "publishing", pollingAttempts: 0, updatedAt: Date.now() })
    return null
  },
})

export const claimPoll = internalMutation({
  args: { deliveryId: v.id("marketDeliveries") },
  handler: async (ctx, args) => {
    const delivery = await ctx.db.get(args.deliveryId)
    if (!delivery || delivery.status !== "publishing" || !delivery.remotePostId) return null
    await ctx.db.patch(delivery._id, {
      pollingAttempts: (delivery.pollingAttempts ?? 0) + 1,
      leaseExpiresAt: Date.now() + 300_000,
    })
    return delivery
  },
})

/** A dispatch that never reported back may have posted; never retry it automatically. */
export const expireLease = internalMutation({
  args: { deliveryId: v.id("marketDeliveries") },
  handler: async (ctx, args) => {
    const delivery = await ctx.db.get(args.deliveryId)
    if (
      !delivery ||
      delivery.status !== "publishing" ||
      !delivery.leaseExpiresAt ||
      delivery.leaseExpiresAt > Date.now()
    )
      return null
    await ctx.db.patch(delivery._id, {
      status: "unknown",
      leaseExpiresAt: undefined,
      error:
        "Publishing was interrupted after dispatch began. Check the network before retrying to avoid a duplicate post.",
      updatedAt: Date.now(),
    })
    await refresh(ctx, delivery.postId)
    return null
  },
})

// Media

export const uploadUrl = internalMutation({ args: {}, handler: async (ctx) => await ctx.storage.generateUploadUrl() })

export const registerMedia = internalMutation({
  args: { user: v.string(), storageId: v.id("_storage"), name: v.string() },
  handler: async (ctx, args) => {
    const file = await ctx.db.system.get(args.storageId)
    if (!file) throw new Error("Upload not found.")
    const contentType = file.contentType ?? ""
    if (!/^(image\/(png|jpeg|gif|webp)|video\/(mp4|quicktime|webm))$/.test(contentType)) {
      await ctx.storage.delete(args.storageId)
      throw new Error("Use a PNG, JPEG, GIF or WebP image, or an MP4, MOV or WebM video.")
    }
    if (
      await ctx.db
        .query("marketMedia")
        .withIndex("by_storage", (q) => q.eq("storageId", args.storageId))
        .unique()
    )
      throw new Error("This upload is already registered.")
    return await ctx.db.insert("marketMedia", {
      user: args.user,
      storageId: args.storageId,
      contentType,
      name: args.name.slice(0, 200) || "Upload",
      size: file.size,
      createdAt: Date.now(),
    })
  },
})

export const listMedia = internalQuery({
  args: { user: v.string() },
  handler: async (ctx, args) => {
    const rows = await ctx.db
      .query("marketMedia")
      .withIndex("by_user", (q) => q.eq("user", args.user))
      .order("desc")
      .take(500)
    return await Promise.all(rows.map(async (row) => ({ ...row, url: await ctx.storage.getUrl(row.storageId) })))
  },
})

export const deleteMedia = internalMutation({
  args: { user: v.string(), storageId: v.id("_storage") },
  handler: async (ctx, args) => {
    const row = await ctx.db
      .query("marketMedia")
      .withIndex("by_storage", (q) => q.eq("storageId", args.storageId))
      .unique()
    if (row?.user !== args.user) throw new Error("Media not found.")
    await ctx.db.delete(row._id)
    await ctx.storage.delete(args.storageId)
    return null
  },
})

// Analytics cache

export const analytics = internalQuery({
  args: { user: v.string(), connectedAccountId: v.string(), days: v.number() },
  handler: async (ctx, args) =>
    await ctx.db
      .query("marketAnalytics")
      .withIndex("by_user_account_days", (q) =>
        q.eq("user", args.user).eq("connectedAccountId", args.connectedAccountId).eq("days", args.days),
      )
      .unique(),
})

export const saveAnalytics = internalMutation({
  args: { user: v.string(), connectedAccountId: v.string(), days: v.number(), data: v.any() },
  handler: async (ctx, args) => {
    const row = await ctx.db
      .query("marketAnalytics")
      .withIndex("by_user_account_days", (q) =>
        q.eq("user", args.user).eq("connectedAccountId", args.connectedAccountId).eq("days", args.days),
      )
      .unique()
    if (row) await ctx.db.patch(row._id, { data: args.data, updatedAt: Date.now() })
    else await ctx.db.insert("marketAnalytics", { ...args, updatedAt: Date.now() })
    return null
  },
})

/** Published post IDs per account, for X analytics, which reports per post. */
export const publishedIds = internalQuery({
  args: { user: v.string(), connectedAccountId: v.string(), from: v.number() },
  handler: async (ctx, args) => {
    const posts = await ctx.db
      .query("marketPosts")
      .withIndex("by_user_and_scheduled", (q) => q.eq("user", args.user).gte("scheduledFor", args.from))
      .take(1000)
    const ids = await Promise.all(
      posts.map(async (post) =>
        (await deliveries(ctx, post._id))
          .filter(
            (item) =>
              item.connectedAccountId === args.connectedAccountId && item.status === "published" && item.remotePostId,
          )
          .map((item) => item.remotePostId!),
      ),
    )
    return [...new Set(ids.flat())]
  },
})

async function clearAnalytics(ctx: MutationCtx, user: string, connectedAccountId: string) {
  const rows = await ctx.db
    .query("marketAnalytics")
    .withIndex("by_user_account_days", (q) => q.eq("user", user).eq("connectedAccountId", connectedAccountId))
    .take(10)
  await Promise.all(rows.map((row) => ctx.db.delete(row._id)))
}
