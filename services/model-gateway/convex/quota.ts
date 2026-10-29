import { v, ConvexError } from "convex/values"
import { internalMutation } from "./_generated/server"
import { limit } from "./lib/limits"

// Internal: only the gateway HTTP actions in this deployment can reserve or settle calls.
export const reserve = internalMutation({
  args: { user: v.string(), step: v.string() },
  returns: v.object({ remaining: v.number() }),
  handler: async (ctx, args) => {
    if (!/^msg_[a-zA-Z0-9_-]{1,120}$/.test(args.step)) throw new ConvexError("invalid step")
    const user = args.user
    const now = Date.now()
    const day = new Date(now).toISOString().slice(0, 10)
    const existing = await ctx.db.query("calls").withIndex("by_user_and_step", (q) => q.eq("user", user).eq("step", args.step)).unique()
    // Retrying a step never invokes the model again, including after completion.
    if (existing) throw new ConvexError("duplicate step")
    const usage = await ctx.db.query("usage").withIndex("by_user_and_day", (q) => q.eq("user", user).eq("day", day)).unique()
    const active = await ctx.db.query("calls").withIndex("by_status_and_expires", (q) => q.eq("status", "active").gt("expires", now)).take(101)
    const pool = await ctx.db.query("pool").withIndex("by_key", (q) => q.eq("key", "nvidia")).unique()
    const minute = Math.floor(now / 60000)
    const minuteCalls = pool?.minute === minute ? pool.calls : 0
    const verdict = limit({ calls: usage?.calls ?? 0, userActive: active.some((call) => call.user === user), active: active.length, minuteCalls,
      maxActive: Number(process.env.FREE_MAX_CONCURRENT ?? 2), maxMinute: Number(process.env.FREE_REQUESTS_PER_MINUTE ?? 20), blockedUntil: pool?.blockedUntil ?? 0, now })
    if (verdict) throw new ConvexError(verdict)
    if (usage) await ctx.db.patch(usage._id, { calls: usage.calls + 1 })
    else await ctx.db.insert("usage", { user, day, calls: 1 })
    if (pool) await ctx.db.patch(pool._id, { minute, calls: minuteCalls + 1 })
    else await ctx.db.insert("pool", { key: "nvidia", minute, calls: 1 })
    // Lease outlives the gateway's hard 270-second request deadline.
    await ctx.db.insert("calls", { user, step: args.step, expires: now + 330000, status: "active" })
    return { remaining: 99 - (usage?.calls ?? 0) }
  },
})
export const finish = internalMutation({
  args: { user: v.string(), step: v.string(), retryAfter: v.optional(v.number()) },
  returns: v.null(),
  handler: async (ctx, args) => {
    const call = await ctx.db.query("calls").withIndex("by_user_and_step", (q) => q.eq("user", args.user).eq("step", args.step)).unique()
    if (!call) throw new ConvexError("unknown step")
    if (call.status === "finished") return null
    await ctx.db.patch(call._id, { status: "finished", expires: Date.now() })
    if (args.retryAfter !== undefined) {
      const pool = await ctx.db.query("pool").withIndex("by_key", (q) => q.eq("key", "nvidia")).unique()
      if (pool) await ctx.db.patch(pool._id, { blockedUntil: Math.max(pool.blockedUntil ?? 0, Date.now() + Math.max(1, Math.min(3600, args.retryAfter)) * 1000) })
    }
    return null
  },
})
