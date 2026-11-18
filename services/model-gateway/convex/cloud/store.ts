// Cloud workspace records and usage. The HTTP layer resolves the user; the worker drives the cryzo-cloud Worker.
import { v } from "convex/values"
import type { Doc } from "../_generated/dataModel"
import { internalMutation, internalQuery, type MutationCtx, type QueryCtx } from "../_generated/server"
import { allowance, caps, day, month, sandboxName } from "../lib/cloud"

type Ctx = QueryCtx | MutationCtx

async function workspace(ctx: Ctx, user: string, project: string) {
  return await ctx.db
    .query("cloudWorkspaces")
    .withIndex("by_user_and_project", (q) => q.eq("user", user).eq("project", project))
    .unique()
}

async function usage(ctx: Ctx, user: string, now: number) {
  const today = await ctx.db
    .query("cloudUsage")
    .withIndex("by_user_and_day", (q) => q.eq("user", user).eq("day", day(now)))
    .unique()
  const budget = await ctx.db
    .query("cloudBudget")
    .withIndex("by_month", (q) => q.eq("month", month(now)))
    .unique()
  return { usedToday: today?.minutes ?? 0, usedMonth: budget?.minutes ?? 0 }
}

/** Adds the running time since the last count to the user's day and the service's month. */
async function account(ctx: MutationCtx, row: Doc<"cloudWorkspaces">, now: number) {
  if (row.status !== "running" || row.accountedAt === undefined) return
  const minutes = Math.max(0, now - row.accountedAt) / 60_000
  await ctx.db.patch(row._id, { accountedAt: now })
  if (minutes === 0) return
  const today = await ctx.db
    .query("cloudUsage")
    .withIndex("by_user_and_day", (q) => q.eq("user", row.user).eq("day", day(now)))
    .unique()
  if (today) await ctx.db.patch(today._id, { minutes: today.minutes + minutes })
  else await ctx.db.insert("cloudUsage", { user: row.user, day: day(now), minutes })
  const budget = await ctx.db
    .query("cloudBudget")
    .withIndex("by_month", (q) => q.eq("month", month(now)))
    .unique()
  if (budget) await ctx.db.patch(budget._id, { minutes: budget.minutes + minutes })
  else await ctx.db.insert("cloudBudget", { month: month(now), minutes })
}

/** Checks the caps and claims the workspace for starting. Returns its sandbox and the minutes allowed. */
export const reserve = internalMutation({
  args: { user: v.string(), project: v.string() },
  handler: async (ctx, args) => {
    const now = Date.now()
    const allowed = allowance({ ...(await usage(ctx, args.user, now)), caps: caps() })
    if (!allowed.ok) throw new Error(allowed.reason)
    const existing = await workspace(ctx, args.user, args.project)
    const sandbox = existing?.sandbox ?? (await sandboxName(args.user, args.project))
    if (existing) {
      await account(ctx, existing, now)
      await ctx.db.patch(existing._id, { status: "starting" })
    } else await ctx.db.insert("cloudWorkspaces", { user: args.user, project: args.project, sandbox, status: "starting" })
    return { sandbox, minutes: allowed.minutes }
  },
})

export const started = internalMutation({
  args: { user: v.string(), project: v.string(), url: v.string(), previews: v.record(v.string(), v.string()) },
  handler: async (ctx, args) => {
    const row = await workspace(ctx, args.user, args.project)
    if (!row) throw new Error("This cloud workspace was removed while it started.")
    const now = Date.now()
    await ctx.db.patch(row._id, { status: "running", url: args.url, previews: args.previews, startedAt: now, lastActiveAt: now, accountedAt: now })
  },
})

/** Counts usage; with `active`, also records that the desktop is still using the workspace. */
export const heartbeat = internalMutation({
  args: { user: v.string(), project: v.string(), active: v.boolean() },
  handler: async (ctx, args) => {
    const row = await workspace(ctx, args.user, args.project)
    if (!row || row.status !== "running") return { running: false as const }
    const now = Date.now()
    await account(ctx, row, now)
    if (args.active) await ctx.db.patch(row._id, { lastActiveAt: now })
    const allowed = allowance({ ...(await usage(ctx, args.user, now)), caps: caps() })
    return { running: true as const, sandbox: row.sandbox, lastActiveAt: args.active ? now : (row.lastActiveAt ?? now), allowed }
  },
})

export const stopped = internalMutation({
  args: { user: v.string(), project: v.string() },
  handler: async (ctx, args) => {
    const row = await workspace(ctx, args.user, args.project)
    if (!row) return
    await account(ctx, row, Date.now())
    await ctx.db.patch(row._id, { status: "stopped", url: undefined, previews: undefined, accountedAt: undefined })
  },
})

export const get = internalQuery({
  args: { user: v.string(), project: v.string() },
  handler: async (ctx, args) => {
    const row = await workspace(ctx, args.user, args.project)
    const now = Date.now()
    const used = await usage(ctx, args.user, now)
    return {
      status: row?.status ?? "stopped",
      url: row?.url ?? null,
      previews: row?.previews ?? {},
      usedToday: Math.round(used.usedToday * 10) / 10,
      dailyMinutes: caps().dailyMinutes,
    }
  },
})
