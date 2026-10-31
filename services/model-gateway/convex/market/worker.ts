"use node"
// Composio calls for Marketing: connections, publishing and analytics.
// Publishing logic follows cryzo.me's socialWorker (Imma2013/cryzo-v11).
import { Composio } from "@composio/core"
import { v } from "convex/values"
import { internal } from "../_generated/api"
import { internalAction } from "../_generated/server"
import {
  authConfig,
  channelForToolkit,
  graphMetrics,
  identifier,
  numberValue,
  payload,
  toolkits,
  youtubeMetrics,
  type Channel,
  type Metric,
  type Series,
} from "../lib/market"
import { channel } from "../schema"

let client: Composio | undefined
function composio() {
  const apiKey = process.env.COMPOSIO_API_KEY?.trim()
  if (!apiKey) throw new Error("Marketing is not configured: COMPOSIO_API_KEY is missing.")
  return (client ??= new Composio({ apiKey }))
}

type Execution = { data: unknown; error?: string | null; successful?: boolean; logId?: string }

async function session(user: string, channel: Channel, connectedAccountId: string) {
  const toolkit = toolkits[channel]
  const auth = authConfig(channel)
  return await composio().create(user, {
    toolkits: [toolkit],
    ...(auth.id ? { authConfigs: { [toolkit]: auth.id } } : {}),
    connectedAccounts: { [toolkit]: connectedAccountId },
  })
}

function readable(message: string, channel?: Channel) {
  if (/credits depleted|http_depleted|status["']?\s*:\s*402|payment required/i.test(message))
    return channel === "x"
      ? "X publishing needs API credits on the connected X developer app. Add credits, then retry."
      : "The provider has no delivery credits available. Refill the provider balance, then retry."
  return message
}

export const connect = internalAction({
  args: { user: v.string(), channel, callbackUrl: v.string() },
  handler: async (_ctx, args) => {
    const auth = authConfig(args.channel)
    if (!auth.id && auth.custom)
      throw new Error(`${args.channel === "x" ? "X" : "TikTok"} needs its own developer app. Set ${auth.name} on the Cryzo cloud deployment.`)
    const request = auth.id
      ? await composio().connectedAccounts.link(args.user, auth.id, { callbackUrl: args.callbackUrl, allowMultiple: true })
      : await (await composio().create(args.user)).authorize(toolkits[args.channel], { callbackUrl: args.callbackUrl })
    if (!request.redirectUrl) throw new Error("Composio did not return a sign-in link.")
    return request.redirectUrl
  },
})

/** Reads the user's active Composio connections into marketAccounts. */
export const sync = internalAction({
  args: { user: v.string() },
  handler: async (ctx, args) => {
    const active: { channel: Channel; connectedAccountId: string; name: string }[] = []
    const seen = new Set<string>()
    let cursor: string | undefined
    do {
      const page = await composio().connectedAccounts.list({ userIds: [args.user], statuses: ["ACTIVE"], limit: 100, cursor })
      for (const item of page.items) {
        const found = channelForToolkit(item.toolkit.slug)
        if (found) active.push({ channel: found, connectedAccountId: item.id, name: `${label(found)} ${item.id.slice(-4)}` })
      }
      cursor = page.nextCursor || undefined
      if (cursor && seen.has(cursor)) throw new Error("Composio connection paging did not finish.")
      if (cursor) seen.add(cursor)
    } while (cursor)
    await ctx.runMutation(internal.market.store.syncAccounts, { user: args.user, active })
    // New accounts get their profile name and picture where the network exposes them.
    const { accounts } = await ctx.runQuery(internal.market.store.listAccounts, { user: args.user })
    for (const account of accounts.filter((item) => item.name.endsWith(item.connectedAccountId.slice(-4)))) {
      const profile = await describe(args.user, account.channel, account.connectedAccountId).catch(() => null)
      if (profile)
        await ctx.runMutation(internal.market.store.updateAccount, {
          user: args.user,
          connectedAccountId: account.connectedAccountId,
          ...profile,
        })
    }
    return null
  },
})

function label(channel: Channel) {
  return { x: "X", facebook: "Facebook", instagram: "Instagram", youtube: "YouTube", tiktok: "TikTok", reddit: "Reddit" }[channel]
}

async function describe(user: string, channel: Channel, connectedAccountId: string) {
  const run = await session(user, channel, connectedAccountId)
  const call = async (slug: string, args: Record<string, unknown>) => {
    const result = (await run.execute(slug, args)) as Execution
    if (result.error) throw new Error(result.error)
    return payload(result.data)
  }
  const pick = (data: any, names: string[], pictures: string[]) => {
    const find = (keys: string[]) => {
      for (const key of keys) {
        const value = key.split(".").reduce((item: any, part) => item?.[part], data)
        if (typeof value === "string" && value) return value
      }
    }
    const name = find(names)
    return name ? { name: name.slice(0, 100), avatar: find(pictures) } : null
  }
  if (channel === "x") return pick(await call("TWITTER_USER_LOOKUP_ME", {}), ["data.username", "username", "data.name", "name"], ["data.profile_image_url", "profile_image_url"])
  if (channel === "instagram") return pick(await call("INSTAGRAM_GET_USER_INFO", {}), ["username", "name"], ["profile_picture_url"])
  if (channel === "youtube")
    return pick(await call("YOUTUBE_LIST_CHANNELS", { part: "snippet", mine: true }), ["items.0.snippet.title"], ["items.0.snippet.thumbnails.default.url"])
  if (channel === "tiktok")
    return pick(await call("TIKTOK_GET_USER_STATS", { fields: ["display_name", "avatar_url"] }), ["user.display_name", "display_name"], ["user.avatar_url", "avatar_url"])
  if (channel === "reddit") return pick(await call("REDDIT_GET_REDDIT_USER_ABOUT", { username: "me" }), ["name", "data.name"], ["icon_img", "data.icon_img"])
  return pick(await call("FACEBOOK_GET_CURRENT_USER", { fields: "name,picture" }), ["name"], ["picture.data.url"])
}

export const disconnect = internalAction({
  args: { user: v.string(), connectedAccountId: v.string() },
  handler: async (ctx, args) => {
    const account = await ctx.runQuery(internal.market.store.account, args)
    if (!account) throw new Error("Channel not found.")
    await composio().connectedAccounts.delete(args.connectedAccountId)
    await ctx.runMutation(internal.market.store.removeAccount, args)
    return null
  },
})

/** Facebook Pages and Instagram professional accounts the connection can post to. */
export const targets = internalAction({
  args: { user: v.string(), connectedAccountId: v.string() },
  handler: async (ctx, args) => {
    const account = await ctx.runQuery(internal.market.store.account, args)
    if (!account) throw new Error("Channel not found.")
    const run = await session(args.user, account.channel, account.connectedAccountId)
    const call = async (slug: string, input: Record<string, unknown>) => {
      const result = (await run.execute(slug, input)) as Execution
      if (result.error) throw new Error(readable(result.error, account.channel))
      return payload(result.data)
    }
    if (account.channel === "facebook") {
      const found: { id: string; name: string }[] = []
      let after: string | undefined
      for (let page = 0; page < 20; page++) {
        const data = await call("FACEBOOK_LIST_MANAGED_PAGES", { user_id: "me", fields: "id,name", limit: 100, ...(after ? { after } : {}) })
        for (const item of data?.data ?? []) if (item.id && item.name) found.push({ id: String(item.id), name: String(item.name) })
        after = data?.paging?.next ? data.paging.cursors?.after : undefined
        if (!after) break
      }
      return found
    }
    if (account.channel === "instagram") {
      const data = await call("INSTAGRAM_GET_USER_INFO", {})
      const id = identifier(data, ["id", "user_id", "ig_user_id"])
      return id ? [{ id, name: String(data?.username ?? data?.name ?? "Instagram account") }] : []
    }
    return []
  },
})

export const publishPost = internalAction({
  args: { postId: v.id("marketPosts") },
  handler: async (ctx, args) => {
    await ctx.runMutation(internal.market.store.claimPost, args)
    return null
  },
})

export const publishDelivery = internalAction({
  args: { deliveryId: v.id("marketDeliveries") },
  handler: async (ctx, args) => {
    const job = await ctx.runQuery(internal.market.store.deliveryForPublish, args)
    if (!job || !(await ctx.runMutation(internal.market.store.claimDelivery, args))) return null
    const { delivery, post, content, targetId } = job
    const net = delivery.channel
    const toolkit = toolkits[net]
    const options = post.options ?? {}
    const urls = job.media.map((item) => item.url).filter((url): url is string => Boolean(url))
    const types = job.media.map((item) => item.type)
    const video = types.some((type) => type.startsWith("video/"))
    // A failure before the first publishing call is safe to retry; after it, the outcome may be unknown.
    let dispatched = false
    let rejected = false
    let slug: string | undefined
    let logId: string | undefined
    try {
      const run = await session(post.user, net, delivery.connectedAccountId)
      const execute = async (tool: string, input: Record<string, unknown>, publishes = true) => {
        slug = tool
        if (publishes) dispatched = true
        const result = (await run.execute(tool, input)) as Execution
        logId = result.logId
        if (result.error || result.successful === false) {
          rejected = true
          throw new Error(readable(String(result.error || "The network rejected the post."), net))
        }
        return result
      }
      const upload = (tool: string, url: string) => composio().files.upload({ file: url, toolSlug: tool, toolkitSlug: toolkit })
      let result: Execution
      if (net === "x") {
        const ids: string[] = []
        for (const url of urls) {
          const media = await upload("TWITTER_UPLOAD_MEDIA", url)
          const uploaded = await execute("TWITTER_UPLOAD_MEDIA", { media, media_category: "tweet_image" }, false)
          const id = identifier(uploaded.data, ["media_id", "media_id_string", "id"])
          if (!id) throw new Error("X did not confirm the media upload.")
          ids.push(id)
        }
        result = await execute("TWITTER_CREATION_OF_A_POST", { text: content, ...(ids.length ? { media_media_ids: ids } : {}) })
      } else if (net === "reddit") {
        result = await execute("REDDIT_CREATE_REDDIT_POST", {
          subreddit: options.redditCommunity?.replace(/^r\//, ""),
          title: content.split("\n")[0].slice(0, 300),
          text: content,
          kind: "self",
        })
      } else if (net === "facebook") {
        result = video
          ? await execute("FACEBOOK_CREATE_VIDEO_POST", { page_id: targetId, file_url: urls[0], description: content, published: true })
          : urls.length
            ? await execute("FACEBOOK_CREATE_MULTI_PHOTO_POST", { page_id: targetId, photo_urls: urls, message: content })
            : await execute("FACEBOOK_CREATE_POST", { page_id: targetId, message: content, published: true })
      } else if (net === "instagram") {
        const kind = options.instagramPostType ?? "post"
        const container =
          kind === "carousel"
            ? await execute(
                "INSTAGRAM_CREATE_CAROUSEL_CONTAINER",
                {
                  ig_user_id: targetId,
                  caption: content,
                  child_image_urls: urls.filter((_, index) => !types[index].startsWith("video/")),
                  child_video_urls: urls.filter((_, index) => types[index].startsWith("video/")),
                },
                false,
              )
            : await execute(
                "INSTAGRAM_POST_IG_USER_MEDIA",
                {
                  ig_user_id: targetId,
                  ...(kind === "story" ? {} : { caption: content }),
                  ...(video
                    ? { video_url: urls[0], media_type: kind === "story" ? "STORIES" : "REELS" }
                    : { image_url: urls[0], ...(kind === "story" ? { media_type: "STORIES" } : {}) }),
                },
                false,
              )
        const creation = identifier(container.data)
        if (!creation) throw new Error("Instagram did not create a media container.")
        result = await execute("INSTAGRAM_POST_IG_USER_MEDIA_PUBLISH", { ig_user_id: targetId, creation_id: creation, max_wait_seconds: 300 })
      } else if (net === "youtube") {
        const videoIndex = types.findIndex((type) => type.startsWith("video/"))
        const thumbnailIndex = types.findIndex((type) => type.startsWith("image/"))
        const file = await upload("YOUTUBE_UPLOAD_VIDEO", urls[videoIndex])
        result = await execute("YOUTUBE_UPLOAD_VIDEO", {
          title: options.youtubeTitle?.trim() || content.split("\n")[0].slice(0, 100) || "Untitled",
          description: content,
          tags: [],
          categoryId: "22",
          privacyStatus: options.youtubePrivacy || "public",
          videoFilePath: file,
        })
        const videoId = identifier(result.data, ["video_id", "id"])
        if (videoId && thumbnailIndex >= 0) await execute("YOUTUBE_UPDATE_THUMBNAIL", { videoId, thumbnailUrl: urls[thumbnailIndex] }, false)
      } else {
        result = await execute("TIKTOK_PUBLISH_VIDEO", { video_url: urls[0], caption: content, privacy_level: options.tiktokPrivacy || "SELF_ONLY" })
        const publishId = identifier(result.data, ["publish_id"])
        if (!publishId) throw new Error("TikTok did not return a publishing ID.")
        await ctx.runMutation(internal.market.store.markProcessing, {
          deliveryId: delivery._id,
          remotePostId: publishId,
          providerLogId: logId,
          toolSlug: "TIKTOK_PUBLISH_VIDEO",
        })
        await ctx.scheduler.runAfter(5000, internal.market.worker.checkTikTok, { deliveryId: delivery._id })
        return null
      }
      const id = identifier(result.data)
      if (!id) throw new Error("The network returned no post ID. Check it before retrying.")
      await ctx.runMutation(internal.market.store.markPublished, { deliveryId: delivery._id, toolSlug: slug!, providerLogId: logId, remotePostId: id })
    } catch (error) {
      await ctx.runMutation(internal.market.store.markFailed, {
        deliveryId: delivery._id,
        error: error instanceof Error ? error.message : "Publishing failed.",
        outcomeUnknown: dispatched && !rejected,
        toolSlug: slug,
        providerLogId: logId,
      })
    }
    return null
  },
})

export const checkTikTok = internalAction({
  args: { deliveryId: v.id("marketDeliveries") },
  handler: async (ctx, args) => {
    const delivery = await ctx.runMutation(internal.market.store.claimPoll, args)
    if (!delivery) return null
    try {
      const run = await session(delivery.user, "tiktok", delivery.connectedAccountId)
      const result = (await run.execute("TIKTOK_FETCH_PUBLISH_STATUS", { publish_id: delivery.remotePostId })) as Execution
      if (result.error) throw new Error(result.error)
      const status = JSON.stringify(result.data).match(/"status"\s*:\s*"([A-Z_]+)"/)?.[1]
      if (status === "PUBLISH_COMPLETE") {
        await ctx.runMutation(internal.market.store.markPublished, {
          deliveryId: delivery._id,
          toolSlug: "TIKTOK_FETCH_PUBLISH_STATUS",
          providerLogId: result.logId,
          remotePostId: delivery.remotePostId,
        })
        return null
      }
      if (status === "FAILED") {
        await ctx.runMutation(internal.market.store.markFailed, {
          deliveryId: delivery._id,
          outcomeUnknown: false,
          error: "TikTok rejected the post. Check the video, privacy setting and account approval.",
          providerLogId: result.logId,
        })
        return null
      }
    } catch {
      // Status checks are safe to repeat; the upload itself is never restarted.
    }
    if ((delivery.pollingAttempts ?? 0) >= 9)
      await ctx.runMutation(internal.market.store.markFailed, {
        deliveryId: delivery._id,
        outcomeUnknown: true,
        error: "TikTok is still processing or its status is unavailable. Check TikTok before retrying.",
      })
    else
      await ctx.scheduler.runAfter(Math.min(60_000, 5000 * 2 ** (delivery.pollingAttempts ?? 0)), internal.market.worker.checkTikTok, args)
    return null
  },
})

type Report = {
  channel: Channel
  connectedAccountId: string
  status: "ready" | "partial" | "target_required" | "permission_required" | "rate_limited" | "unsupported" | "error"
  metrics: Metric[]
  series: Series[]
  note?: string
  updatedAt: number
}

/** Per-account analytics for the last `days`, cached for 15 minutes unless `refresh` is set. */
export const analytics = internalAction({
  args: { user: v.string(), days: v.number(), refresh: v.boolean() },
  handler: async (ctx, args): Promise<Report[]> => {
    const days = [7, 30, 90].includes(args.days) ? args.days : 30
    const { accounts } = await ctx.runQuery(internal.market.store.listAccounts, { user: args.user })
    const reports: Report[] = []
    for (const account of accounts) {
      const cached = await ctx.runQuery(internal.market.store.analytics, { user: args.user, connectedAccountId: account.connectedAccountId, days })
      if (cached && !args.refresh && Date.now() - cached.updatedAt < 15 * 60_000) {
        reports.push(cached.data as Report)
        continue
      }
      const base: Report = { channel: account.channel, connectedAccountId: account.connectedAccountId, status: "ready", metrics: [], series: [], updatedAt: Date.now() }
      const report = await measure(ctx, args.user, account, days, base).catch((error: unknown): Report => {
        const message = error instanceof Error ? error.message : String(error)
        if (/429|rate.?limit|throttl/i.test(message)) return { ...base, status: "rate_limited", note: "The network's rate limit was reached. Try again in a minute." }
        if (/401|403|scope|permission|forbidden|expired/i.test(message))
          return { ...base, status: "permission_required", note: "Reconnect this channel with analytics permissions." }
        return { ...base, status: "error", note: "The network could not return analytics. Retry later." }
      })
      await ctx.runMutation(internal.market.store.saveAnalytics, { user: args.user, connectedAccountId: account.connectedAccountId, days, data: report })
      reports.push(report)
    }
    return reports
  },
})

async function measure(
  ctx: { runQuery: any },
  user: string,
  account: { channel: Channel; connectedAccountId: string; targetId?: string },
  days: number,
  base: Report,
): Promise<Report> {
  const to = new Date()
  const from = new Date(to)
  from.setUTCHours(0, 0, 0, 0)
  from.setUTCDate(from.getUTCDate() - days + 1)
  const run = await session(user, account.channel, account.connectedAccountId)
  const call = async (slug: string, input: Record<string, unknown>) => {
    const result = (await run.execute(slug, input)) as Execution
    if (result.error || result.successful === false) throw new Error(String(result.error || "The network rejected the analytics request."))
    return result
  }
  if (account.channel === "facebook") {
    if (!account.targetId) return { ...base, status: "target_required", note: "Choose the Facebook Page to report on." }
    return {
      ...base,
      ...graphMetrics(
        await call("FACEBOOK_GET_PAGE_INSIGHTS", {
          page_id: account.targetId,
          since: from.toISOString(),
          until: to.toISOString(),
          period: "day",
          metrics: "page_follows,page_daily_follows_unique,page_media_view,page_post_engagements,page_video_views",
        }),
      ),
    }
  }
  if (account.channel === "instagram") {
    const totals = graphMetrics(
      await call("INSTAGRAM_GET_USER_INSIGHTS", {
        since: Math.floor(Math.max(from.getTime(), to.getTime() - 30 * 86_400_000) / 1000),
        until: Math.floor(to.getTime() / 1000),
        period: "day",
        metric: ["views", "likes", "comments", "shares", "saves"],
        metric_type: "total_value",
      }),
    )
    const reach = graphMetrics(
      await call("INSTAGRAM_GET_USER_INSIGHTS", {
        since: Math.floor(Math.max(from.getTime(), to.getTime() - 30 * 86_400_000) / 1000),
        until: Math.floor(to.getTime() / 1000),
        period: "day",
        metric: ["reach"],
        metric_type: "time_series",
      }),
    )
    return {
      ...base,
      status: days > 30 ? "partial" : "ready",
      metrics: [...totals.metrics, ...reach.metrics],
      series: reach.series,
      ...(days > 30 ? { note: "Instagram reports at most the last 30 days." } : {}),
    }
  }
  if (account.channel === "youtube") {
    return {
      ...base,
      ...youtubeMetrics(
        await call("YOUTUBE_QUERY_ANALYTICS", {
          ids: "channel==MINE",
          startDate: from.toISOString().slice(0, 10),
          endDate: to.toISOString().slice(0, 10),
          metrics: "views,likes,comments,estimatedMinutesWatched,averageViewDuration,subscribersGained,subscribersLost",
          dimensions: "day",
          sort: "day",
          maxResults: 200,
        }),
      ),
    }
  }
  if (account.channel === "x") {
    const ids: string[] = (
      await ctx.runQuery(internal.market.store.publishedIds, { user, connectedAccountId: account.connectedAccountId, from: from.getTime() })
    ).filter((id: string) => /^\d{1,19}$/.test(id))
    if (!ids.length) return { ...base, status: "partial", note: "No posts published through Cryzo in this period yet." }
    const rows = payload(await call("TWITTER_POST_LOOKUP_BY_POST_IDS", { ids: ids.slice(0, 100), tweet_fields: ["public_metrics", "created_at"] }))
    const metrics = (Array.isArray(rows?.data) ? rows.data : []).map((row: any) => row.public_metrics ?? {})
    const sum = (key: string) => metrics.reduce((total: number, row: any) => total + (numberValue(row[key]) ?? 0), 0)
    return {
      ...base,
      status: "partial",
      note: "Lifetime public counters for posts published through Cryzo.",
      metrics: ["impression_count", "like_count", "reply_count", "retweet_count", "quote_count", "bookmark_count"].map((key) => ({
        key,
        value: sum(key),
        window: "lifetime" as const,
      })),
    }
  }
  if (account.channel === "tiktok") {
    const user = payload(await call("TIKTOK_GET_USER_STATS", { fields: ["follower_count", "following_count", "likes_count", "video_count"] }))
    const stats = user?.user ?? user
    return {
      ...base,
      status: "partial",
      note: "Account totals as of now.",
      metrics: ["follower_count", "likes_count", "video_count"].flatMap((key) => {
        const value = numberValue(stats?.[key])
        return value === null ? [] : [{ key, value, window: "snapshot" as const }]
      }),
    }
  }
  return { ...base, status: "unsupported", note: "Reddit does not provide account analytics through this connection." }
}
