// Published app records. The HTTP layer resolves the user; cloudflare.ts deploys.
import { v } from "convex/values"
import { internalMutation, internalQuery } from "../_generated/server"
import { FREE_APPS } from "../lib/apps"

/** Reserves an address for the user, or explains why they can't have it. */
export const claim = internalMutation({
  args: { user: v.string(), slug: v.string(), script: v.string() },
  handler: async (ctx, args) => {
    const existing = await ctx.db
      .query("cloudApps")
      .withIndex("by_slug", (q) => q.eq("slug", args.slug))
      .unique()
    if (existing && existing.user !== args.user) throw new Error("That address is taken. Choose another one.")
    if (existing) return existing
    const owned = await ctx.db
      .query("cloudApps")
      .withIndex("by_user", (q) => q.eq("user", args.user))
      .take(FREE_APPS)
    if (owned.length >= FREE_APPS)
      throw new Error(`The free plan includes ${FREE_APPS} published app. Unpublish ${owned[0].slug} to publish a different one.`)
    const id = await ctx.db.insert("cloudApps", { user: args.user, slug: args.slug, script: args.script, secret: false })
    return (await ctx.db.get(id))!
  },
})

export const update = internalMutation({
  args: {
    user: v.string(),
    slug: v.string(),
    database: v.optional(v.string()),
    secret: v.optional(v.boolean()),
    url: v.optional(v.string()),
    publishedAt: v.optional(v.number()),
  },
  handler: async (ctx, args) => {
    const row = await ctx.db
      .query("cloudApps")
      .withIndex("by_slug", (q) => q.eq("slug", args.slug))
      .unique()
    if (!row || row.user !== args.user) throw new Error("This app was unpublished while it was being published.")
    const { user: _user, slug: _slug, ...fields } = args
    await ctx.db.patch(row._id, fields)
  },
})

export const get = internalQuery({
  args: { user: v.string(), slug: v.string() },
  handler: async (ctx, args) => {
    const row = await ctx.db
      .query("cloudApps")
      .withIndex("by_slug", (q) => q.eq("slug", args.slug))
      .unique()
    return row && row.user === args.user ? row : null
  },
})

export const list = internalQuery({
  args: { user: v.string() },
  handler: async (ctx, args) => {
    const rows = await ctx.db
      .query("cloudApps")
      .withIndex("by_user", (q) => q.eq("user", args.user))
      .take(50)
    return rows.map((row) => ({ slug: row.slug, url: row.url ?? null, publishedAt: row.publishedAt ?? null }))
  },
})

export const remove = internalMutation({
  args: { user: v.string(), slug: v.string() },
  handler: async (ctx, args) => {
    const row = await ctx.db
      .query("cloudApps")
      .withIndex("by_slug", (q) => q.eq("slug", args.slug))
      .unique()
    if (row && row.user === args.user) await ctx.db.delete(row._id)
  },
})
