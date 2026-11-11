"use node"
// Network calls for Marketing: finishing OAuth sign-ins, refreshing tokens, publishing and analytics.
import { v } from "convex/values"
import { internal } from "../_generated/api"
import type { Doc } from "../_generated/dataModel"
import { internalAction, type ActionCtx } from "../_generated/server"
import { open, seal } from "../lib/crypto"
import { labels, type Channel, type Metric, type Series } from "../lib/market"
import { client, providers, ProviderError, redirect, request, type Creator, type Job, type Send, type Tokens } from "../lib/providers"
import { channel } from "../schema"

type Account = Doc<"marketAccounts">

/** Exchanges the OAuth code, reads the profile and stores the sealed tokens. */
export const finishConnect = internalAction({
  args: { user: v.string(), channel, code: v.string(), verifier: v.optional(v.string()) },
  handler: async (ctx, args): Promise<string> => {
    const provider = providers[args.channel]
    const app = client(args.channel)
    if (!app) throw new Error(`${labels[args.channel]} is not set up on the Cryzo cloud yet.`)
    const tokens = await provider.exchange({
      client: app,
      redirect: redirect(args.channel),
      code: args.code,
      verifier: args.verifier,
    })
    const profile = await provider.profile(tokens)
    // One Page or business account is chosen automatically; several need the picker in Cryzo.
    const choices = profile.target ? [profile.target] : provider.targets ? await provider.targets(tokens) : []
    const target = choices.length === 1 ? choices[0] : undefined
    return await ctx.runMutation(internal.market.store.upsertAccount, {
      user: args.user,
      channel: args.channel,
      providerUserId: profile.id,
      name: profile.name,
      ...(profile.avatar ? { avatar: profile.avatar } : {}),
      token: await seal(tokens),
      ...(tokens.expiresAt ? { expiresAt: tokens.expiresAt } : {}),
      refreshable: Boolean(tokens.refresh && provider.refresh),
      ...(target
        ? {
            targetId: target.id,
            targetName: target.name,
            ...(target.token ? { targetToken: await seal(target.token) } : {}),
          }
        : {}),
    })
  },
})

export const refreshAccount = internalAction({
  args: { accountId: v.id("marketAccounts") },
  handler: async (ctx, args): Promise<null> => {
    const account: Account | null = await ctx.runQuery(internal.market.store.accountById, args)
    if (!account?.refreshable || account.needsReconnect) return null
    // A later refresh already moved the expiry.
    if (account.expiresAt && account.expiresAt - Date.now() > 15 * 60_000) return null
    await renew(ctx, account).catch(() => null)
    return null
  },
})

/** Pages, organizations or business accounts the connection can post as. Tokens stay on the server. */
export const targets = internalAction({
  args: { user: v.string(), connectedAccountId: v.string() },
  handler: async (ctx, args): Promise<{ id: string; name: string }[]> => {
    const account: Account | null = await ctx.runQuery(internal.market.store.account, args)
    if (!account) throw new Error("Channel not found.")
    const list = providers[account.channel].targets
    if (!list) return []
    return (await list(await fresh(ctx, account))).map((item) => ({ id: item.id, name: item.name }))
  },
})

export const creator = internalAction({
  args: { user: v.string(), connectedAccountId: v.string() },
  handler: async (ctx, args): Promise<Creator | null> => {
    const account: Account | null = await ctx.runQuery(internal.market.store.account, args)
    if (!account) throw new Error("Channel not found.")
    const read = providers[account.channel].creator
    return read ? read(await fresh(ctx, account)) : null
  },
})

export const chooseTarget = internalAction({
  args: { user: v.string(), connectedAccountId: v.string(), targetId: v.string() },
  handler: async (ctx, args): Promise<null> => {
    const account: Account | null = await ctx.runQuery(internal.market.store.account, args)
    if (!account) throw new Error("Channel not found.")
    const list = providers[account.channel].targets
    const found = list ? (await list(await fresh(ctx, account))).find((item) => item.id === args.targetId) : undefined
    if (!found) throw new Error("That page or account is no longer available to this connection.")
    await ctx.runMutation(internal.market.store.setTarget, {
      user: args.user,
      connectedAccountId: args.connectedAccountId,
      targetId: found.id,
      targetName: found.name,
      ...(found.token ? { targetToken: await seal(found.token) } : {}),
    })
    return null
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
  handler: async (ctx, args): Promise<null> => {
    const found = await ctx.runQuery(internal.market.store.deliveryForPublish, args)
    if (!found || !(await ctx.runMutation(internal.market.store.claimDelivery, args))) return null
    const { delivery, post, account } = found
    // A failure before the publishing call is safe to retry; after it, the outcome may be unknown.
    let dispatched = false
    let operation: string | undefined
    const send: Send = async (url, init, publishes = false) => {
      operation = new URL(url).pathname.slice(0, 200)
      if (publishes) dispatched = true
      try {
        return await request(url, init)
      } catch (error) {
        if (publishes && error instanceof ProviderError && error.status < 500) error.rejected = true
        throw error
      }
    }
    try {
      if (!account) throw new Error("This channel was disconnected. Connect it again, then retry.")
      const sent = await providers[delivery.channel].publish(
        await job(ctx, account, { content: found.content, options: post.options ?? {}, media: found.media }),
        send,
      )
      if (sent.pending) {
        await ctx.runMutation(internal.market.store.markProcessing, {
          deliveryId: delivery._id,
          remotePostId: sent.id,
          operation: operation ?? "publish",
        })
        await ctx.scheduler.runAfter(5000, internal.market.worker.checkStatus, { deliveryId: delivery._id })
        return null
      }
      await ctx.runMutation(internal.market.store.markPublished, {
        deliveryId: delivery._id,
        operation: operation ?? "publish",
        remotePostId: sent.id,
        ...(sent.url ? { remoteUrl: sent.url } : {}),
      })
    } catch (error) {
      await ctx.runMutation(internal.market.store.markFailed, {
        deliveryId: delivery._id,
        error: readable(error, delivery.channel),
        outcomeUnknown: dispatched && !(error instanceof ProviderError && error.rejected),
        operation,
      })
    }
    return null
  },
})

/** Polls networks that publish asynchronously (TikTok) until the post is live or rejected. */
export const checkStatus = internalAction({
  args: { deliveryId: v.id("marketDeliveries") },
  handler: async (ctx, args): Promise<null> => {
    const delivery: Doc<"marketDeliveries"> | null = await ctx.runMutation(internal.market.store.claimPoll, args)
    if (!delivery) return null
    const status = providers[delivery.channel].status
    try {
      const account: Account | null = await ctx.runQuery(internal.market.store.account, {
        user: delivery.user,
        connectedAccountId: delivery.connectedAccountId,
      })
      if (account && status) {
        const result = await status(
          await job(ctx, account, { content: "", options: {}, media: [] }),
          delivery.remotePostId!,
          (url, init) => request(url, init),
        )
        if (result.state === "published") {
          await ctx.runMutation(internal.market.store.markPublished, {
            deliveryId: delivery._id,
            operation: "status",
            remotePostId: result.id ?? delivery.remotePostId,
            ...(result.url ? { remoteUrl: result.url } : {}),
          })
          return null
        }
        if (result.state === "failed") {
          await ctx.runMutation(internal.market.store.markFailed, {
            deliveryId: delivery._id,
            outcomeUnknown: false,
            error: result.reason ?? "The network rejected the post.",
          })
          return null
        }
      }
    } catch {
      // Status checks are safe to repeat; the upload itself is never restarted.
    }
    if ((delivery.pollingAttempts ?? 0) >= 9)
      await ctx.runMutation(internal.market.store.markFailed, {
        deliveryId: delivery._id,
        outcomeUnknown: true,
        error: `${labels[delivery.channel]} is still processing or its status is unavailable. Check it before retrying.`,
      })
    else
      await ctx.scheduler.runAfter(
        Math.min(60_000, 5000 * 2 ** (delivery.pollingAttempts ?? 0)),
        internal.market.worker.checkStatus,
        args,
      )
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
  permissionReason?: "approval_pending" | "reconnect_required"
  updatedAt: number
}

/** Per-account analytics for the last `days`, cached for 15 minutes unless `refresh` is set. */
export const analytics = internalAction({
  args: { user: v.string(), days: v.number(), refresh: v.boolean() },
  returns: v.array(
    v.object({
      channel,
      connectedAccountId: v.string(),
      status: v.union(
        v.literal("ready"),
        v.literal("partial"),
        v.literal("target_required"),
        v.literal("permission_required"),
        v.literal("rate_limited"),
        v.literal("unsupported"),
        v.literal("error"),
      ),
      metrics: v.array(
        v.object({
          key: v.string(),
          value: v.number(),
          window: v.union(v.literal("period"), v.literal("snapshot"), v.literal("lifetime")),
        }),
      ),
      series: v.array(
        v.object({ key: v.string(), points: v.array(v.object({ date: v.string(), value: v.number() })) }),
      ),
      note: v.optional(v.string()),
      permissionReason: v.optional(v.union(v.literal("approval_pending"), v.literal("reconnect_required"))),
      updatedAt: v.number(),
    }),
  ),
  handler: async (ctx, args): Promise<Report[]> => {
    const days = [7, 30, 90].includes(args.days) ? args.days : 30
    const { accounts } = await ctx.runQuery(internal.market.store.listAccounts, { user: args.user })
    const reports: Report[] = []
    for (const item of accounts) {
      const cached = await ctx.runQuery(internal.market.store.analytics, {
        user: args.user,
        connectedAccountId: item.connectedAccountId,
        days,
      })
      if (cached && !args.refresh && Date.now() - cached.updatedAt < 15 * 60_000) {
        reports.push(cached.data as Report)
        continue
      }
      const base: Report = {
        channel: item.channel,
        connectedAccountId: item.connectedAccountId,
        status: "ready",
        metrics: [],
        series: [],
        updatedAt: Date.now(),
      }
      const report = await measure(ctx, args.user, item.connectedAccountId, days, base).catch(
        (error: unknown): Report => {
          const message = error instanceof Error ? error.message : String(error)
          if (/429|rate.?limit|throttl/i.test(message))
            return {
              ...base,
              status: "rate_limited",
              note: "The network's rate limit was reached. Try again in a minute.",
            }
          if (/reconnect|401|403|scope|permission|forbidden|expired/i.test(message))
            return {
              ...base,
              status: "permission_required",
              note: "Reconnect this channel with analytics permissions.",
            }
          return { ...base, status: "error", note: "The network could not return analytics. Retry later." }
        },
      )
      await ctx.runMutation(internal.market.store.saveAnalytics, {
        user: args.user,
        connectedAccountId: item.connectedAccountId,
        days,
        data: report,
      })
      reports.push(report)
    }
    return reports
  },
})

async function measure(
  ctx: ActionCtx,
  user: string,
  connectedAccountId: string,
  days: number,
  base: Report,
): Promise<Report> {
  const account: Account | null = await ctx.runQuery(internal.market.store.account, { user, connectedAccountId })
  if (!account) throw new Error("Channel not found.")
  const to = new Date()
  const from = new Date(to)
  from.setUTCHours(0, 0, 0, 0)
  from.setUTCDate(from.getUTCDate() - days + 1)
  const published =
    account.channel === "x"
      ? await ctx.runQuery(internal.market.store.publishedIds, { user, connectedAccountId, from: from.getTime() })
      : []
  const current = await job(ctx, account, { content: "", options: {}, media: [] })
  return { ...base, ...(await providers[account.channel].metrics({ ...current, days, from, to, published })) }
}

/** Opens the account's tokens, refreshing them first when they are about to expire. */
async function job(ctx: ActionCtx, account: Account, post: Pick<Job, "content" | "options" | "media">): Promise<Job> {
  return {
    ...post,
    tokens: await fresh(ctx, account),
    ...(account.targetId ? { targetId: account.targetId } : {}),
    ...(account.targetToken ? { targetToken: await open<string>(account.targetToken) } : {}),
  }
}

async function fresh(ctx: ActionCtx, account: Account) {
  if (account.needsReconnect) throw new Error(`Reconnect ${labels[account.channel]} in Cryzo Marketing.`)
  const tokens = await open<Tokens>(account.token)
  if (!account.expiresAt || account.expiresAt - Date.now() > 5 * 60_000) return tokens
  if (!account.refreshable) {
    await ctx.runMutation(internal.market.store.flagReconnect, { accountId: account._id })
    throw new Error(`The ${labels[account.channel]} connection expired. Reconnect it in Cryzo Marketing.`)
  }
  return await renew(ctx, account, tokens)
}

async function renew(ctx: ActionCtx, account: Account, current?: Tokens) {
  const provider = providers[account.channel]
  const app = client(account.channel)
  if (!app || !provider.refresh) throw new Error(`${labels[account.channel]} is not set up on the Cryzo cloud.`)
  try {
    const next = await provider.refresh(current ?? (await open<Tokens>(account.token)), app)
    await ctx.runMutation(internal.market.store.saveTokens, {
      accountId: account._id,
      token: await seal(next),
      ...(next.expiresAt ? { expiresAt: next.expiresAt } : {}),
    })
    return next
  } catch (error) {
    // A refused refresh needs the user; network or server errors are retried by the next attempt.
    if (error instanceof ProviderError && error.status < 500)
      await ctx.runMutation(internal.market.store.flagReconnect, { accountId: account._id })
    throw new Error(`The ${labels[account.channel]} connection needs to be renewed. Reconnect it in Cryzo Marketing.`)
  }
}

function readable(error: unknown, channel: Channel) {
  const message = error instanceof Error ? error.message : "Publishing failed."
  if (/credits depleted|CreditsDepleted|status["']?\s*:\s*402|payment required/i.test(message))
    return channel === "x"
      ? "X publishing needs API credits on the Cryzo X developer app."
      : "The network has no publishing credits available."
  return message.slice(0, 1000)
}
