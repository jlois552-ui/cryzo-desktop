// Remote control records. The HTTP layer (remote/http.ts) resolves the caller and hashes every
// secret before it reaches these functions; none is callable from clients directly.
import { v } from "convex/values"
import type { Id } from "../_generated/dataModel"
import { internalMutation, internalQuery, type MutationCtx } from "../_generated/server"

export const limits = { hostsPerUser: 5, clientsPerHost: 10, pairingMs: 5 * 60_000 }

export const createHost = internalMutation({
  args: { user: v.string(), name: v.string(), secretHash: v.string() },
  handler: async (ctx, args) => {
    const hosts = await ctx.db
      .query("remoteHosts")
      .withIndex("by_user", (q) => q.eq("user", args.user))
      .take(limits.hostsPerUser + 1)
    if (hosts.length >= limits.hostsPerUser) throw new Error(`Remote control is on for ${limits.hostsPerUser} computers already. Turn it off on one first.`)
    return await ctx.db.insert("remoteHosts", { ...args, createdAt: Date.now() })
  },
})

export const removeHost = internalMutation({
  args: { user: v.string(), host: v.id("remoteHosts") },
  handler: async (ctx, args) => {
    await ownedHost(ctx, args.user, args.host)
    for (const client of await ctx.db.query("remoteClients").withIndex("by_host", (q) => q.eq("host", args.host)).take(100))
      await ctx.db.delete(client._id)
    for (const pairing of await ctx.db.query("remotePairings").withIndex("by_host", (q) => q.eq("host", args.host)).take(100))
      await ctx.db.delete(pairing._id)
    await ctx.db.delete(args.host)
  },
})

/** Starts pairing: one live code per host, so a new code replaces the previous one. */
export const pair = internalMutation({
  args: { user: v.string(), host: v.id("remoteHosts"), codeHash: v.string() },
  handler: async (ctx, args) => {
    await ownedHost(ctx, args.user, args.host)
    for (const pairing of await ctx.db.query("remotePairings").withIndex("by_host", (q) => q.eq("host", args.host)).take(100))
      await ctx.db.delete(pairing._id)
    const expiresAt = Date.now() + limits.pairingMs
    await ctx.db.insert("remotePairings", { ...args, expiresAt })
    return { expiresAt }
  },
})

/** Redeems a pairing code once and records the new client. */
export const claim = internalMutation({
  args: { codeHash: v.string(), name: v.string(), secretHash: v.string() },
  handler: async (ctx, args) => {
    const pairing = await ctx.db
      .query("remotePairings")
      .withIndex("by_code", (q) => q.eq("codeHash", args.codeHash))
      .unique()
    if (!pairing || pairing.expiresAt <= Date.now()) {
      if (pairing) await ctx.db.delete(pairing._id)
      throw new Error("This pairing code expired or was already used. Show a new one in Cryzo on your computer.")
    }
    await ctx.db.delete(pairing._id)
    const clients = await ctx.db
      .query("remoteClients")
      .withIndex("by_host", (q) => q.eq("host", pairing.host))
      .take(limits.clientsPerHost + 1)
    if (clients.length >= limits.clientsPerHost) throw new Error(`This computer already has ${limits.clientsPerHost} paired devices. Remove one in Cryzo first.`)
    const client = await ctx.db.insert("remoteClients", {
      user: pairing.user,
      host: pairing.host,
      name: args.name,
      secretHash: args.secretHash,
      createdAt: Date.now(),
    })
    return { host: pairing.host, client }
  },
})

export const clients = internalQuery({
  args: { user: v.string(), host: v.id("remoteHosts") },
  handler: async (ctx, args) => {
    const host = await ctx.db.get(args.host)
    if (!host || host.user !== args.user) throw new Error("This computer is not set up for remote control.")
    const rows = await ctx.db
      .query("remoteClients")
      .withIndex("by_host", (q) => q.eq("host", args.host))
      .take(limits.clientsPerHost)
    return rows.map((row) => ({ id: row._id, name: row.name, createdAt: row.createdAt, lastSeenAt: row.lastSeenAt ?? null }))
  },
})

export const revoke = internalMutation({
  args: { user: v.string(), client: v.id("remoteClients") },
  handler: async (ctx, args) => {
    const client = await ctx.db.get(args.client)
    if (!client || client.user !== args.user) throw new Error("That device is not paired.")
    await ctx.db.delete(args.client)
    return { host: client.host }
  },
})

/** Checks a host's or client's secret before the HTTP layer signs a relay ticket. */
export const authenticate = internalMutation({
  args: {
    role: v.union(v.literal("host"), v.literal("client")),
    id: v.string(),
    secretHash: v.string(),
  },
  handler: async (ctx, args) => {
    const now = Date.now()
    if (args.role === "host") {
      const id = ctx.db.normalizeId("remoteHosts", args.id)
      const host = id ? await ctx.db.get(id) : null
      if (!host || host.secretHash !== args.secretHash) return null
      await ctx.db.patch(host._id, { lastSeenAt: now })
      return { host: host._id as string }
    }
    const id = ctx.db.normalizeId("remoteClients", args.id)
    const client = id ? await ctx.db.get(id) : null
    if (!client || client.secretHash !== args.secretHash) return null
    await ctx.db.patch(client._id, { lastSeenAt: now })
    return { host: client.host as string, client: client._id as string }
  },
})

async function ownedHost(ctx: MutationCtx, user: string, host: Id<"remoteHosts">) {
  const row = await ctx.db.get(host)
  if (!row || row.user !== user) throw new Error("This computer is not set up for remote control.")
  return row
}
