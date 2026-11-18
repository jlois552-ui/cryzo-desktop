// Remote control HTTP API. The desktop engine manages its host and pairings with the user's
// Better Auth bearer token. Phones and browsers have no Cryzo session: they redeem a pairing
// code once, then trade their device secret for short-lived relay tickets.
import type { HttpRouter } from "convex/server"
import { internal } from "../_generated/api"
import type { Id } from "../_generated/dataModel"
import { httpAction, type ActionCtx } from "../_generated/server"
import { account } from "../gateway"
import { random } from "../lib/crypto"
import { sign } from "../lib/ticket"

const ticketMs = 5 * 60_000
// Unambiguous characters: no 0/O, 1/I/L.
const alphabet = "ABCDEFGHJKMNPQRSTUVWXYZ23456789"

const cors = {
  "Access-Control-Allow-Origin": "*",
  "Access-Control-Allow-Methods": "POST, OPTIONS",
  "Access-Control-Allow-Headers": "Content-Type",
  "Access-Control-Max-Age": "600",
}
const headers = { "Content-Type": "application/json", "Cache-Control": "no-store", "X-Content-Type-Options": "nosniff" }
const reply = (body: unknown, status = 200, extra: Record<string, string> = {}) =>
  new Response(JSON.stringify(body), { status, headers: { ...headers, ...extra } })

type Input = Record<string, unknown>

function owner(handler: (ctx: ActionCtx, user: string, input: Input) => Promise<unknown>) {
  return httpAction(async (ctx, request) => {
    const user = await account(ctx, request)
    if (!user) return reply({ error: "Sign in to Cryzo to use remote control.", code: "sign_in" }, 401)
    const input = await body(request)
    if (!input) return reply({ error: "Invalid request body." }, 400)
    return run(() => handler(ctx, user, input))
  })
}

function open(handler: (ctx: ActionCtx, input: Input) => Promise<unknown>) {
  return httpAction(async (ctx, request) => {
    const input = await body(request)
    if (!input) return reply({ error: "Invalid request body." }, 400, cors)
    return run(() => handler(ctx, input), cors)
  })
}

async function run(work: () => Promise<unknown>, extra: Record<string, string> = {}) {
  try {
    return reply({ data: (await work()) ?? null }, 200, extra)
  } catch (error) {
    const message = error instanceof Error ? error.message : "Request failed."
    const status = error instanceof Denied ? 401 : 400
    return reply({ error: message.replace(/^\[.*?\]\s*/, "").replace(/^Uncaught Error:\s*/, "").split("\n")[0] }, status, extra)
  }
}

async function body(request: Request): Promise<Input | null> {
  if (request.method === "GET") return Object.fromEntries(new URL(request.url).searchParams)
  if (Number(request.headers.get("content-length") ?? 0) > 16_384) return null
  const value = await request.json().catch(() => null)
  return value && typeof value === "object" && !Array.isArray(value) ? (value as Input) : null
}

class Denied extends Error {}

const text = (value: unknown, name: string, max = 200) => {
  if (typeof value !== "string" || !value.trim() || value.length > max) throw new Error(`Invalid ${name}.`)
  return value.trim()
}

export async function digest(value: string) {
  const bytes = new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(value)))
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("")
}

/** Codes are typed on phones, so dashes, spaces and case are ignored. */
export function normalizeCode(value: string) {
  return value.toUpperCase().replace(/[^A-Z0-9]/g, "")
}

function code() {
  const bytes = crypto.getRandomValues(new Uint8Array(8))
  const raw = Array.from(bytes, (byte) => alphabet[byte % alphabet.length]).join("")
  return `${raw.slice(0, 4)}-${raw.slice(4)}`
}

function config() {
  const key = process.env.REMOTE_TICKET_KEY?.trim()
  const relay = process.env.REMOTE_RELAY_URL?.trim()
  if (!key || !relay) throw new Error("Remote control is not set up on the Cryzo cloud yet.")
  return { key, relay, web: process.env.REMOTE_WEB_URL?.trim() }
}

// Revoking a device also drops its open relay connection; its ticket would otherwise last up to 5 minutes.
async function kick(host: string, client: string) {
  const { key, relay } = config()
  const url = new URL(relay)
  url.protocol = url.protocol === "ws:" ? "http:" : "https:"
  url.pathname = "/kick"
  url.search = ""
  const ticket = await sign({ role: "kick", host, client, exp: Date.now() + 60_000 }, key)
  await fetch(url, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ ticket }), signal: AbortSignal.timeout(5000) }).catch(
    () => undefined,
  )
}

export function remoteRoutes(http: HttpRouter) {
  const post = (path: string, handler: ReturnType<typeof httpAction>) => http.route({ path: `/remote/${path}`, method: "POST", handler })

  post(
    "host",
    owner(async (ctx, user, input) => {
      config()
      const secret = random(32)
      const host = await ctx.runMutation(internal.remote.store.createHost, { user, name: text(input.name, "computer name", 80), secretHash: await digest(secret) })
      return { host, secret }
    }),
  )
  post(
    "host/remove",
    owner((ctx, user, input) => ctx.runMutation(internal.remote.store.removeHost, { user, host: text(input.host, "computer") as Id<"remoteHosts"> })),
  )
  post(
    "pair",
    owner(async (ctx, user, input) => {
      const { web } = config()
      const value = code()
      const { expiresAt } = await ctx.runMutation(internal.remote.store.pair, {
        user,
        host: text(input.host, "computer") as Id<"remoteHosts">,
        codeHash: await digest(normalizeCode(value)),
      })
      return { code: value, expiresAt, url: web ? `${web.replace(/\/+$/, "")}/?pair=${encodeURIComponent(value)}` : null }
    }),
  )
  http.route({
    path: "/remote/clients",
    method: "GET",
    handler: owner((ctx, user, input) => ctx.runQuery(internal.remote.store.clients, { user, host: text(input.host, "computer") as Id<"remoteHosts"> })),
  })
  post(
    "revoke",
    owner(async (ctx, user, input) => {
      const client = text(input.client, "device")
      const { host } = await ctx.runMutation(internal.remote.store.revoke, { user, client: client as Id<"remoteClients"> })
      await kick(host, client)
      return { revoked: true }
    }),
  )
  post(
    "claim",
    open(async (ctx, input) => {
      const secret = random(32)
      const claimed = await ctx.runMutation(internal.remote.store.claim, {
        codeHash: await digest(normalizeCode(text(input.code, "pairing code", 20))),
        name: text(input.name ?? "Phone", "device name", 80),
        secretHash: await digest(secret),
      })
      return { host: claimed.host, client: claimed.client, secret }
    }),
  )
  post(
    "ticket",
    open(async (ctx, input) => {
      const { key, relay } = config()
      const role = input.role === "host" ? "host" : input.role === "client" ? "client" : null
      if (!role) throw new Error("Invalid role.")
      const found = await ctx.runMutation(internal.remote.store.authenticate, {
        role,
        id: text(input.id, "device"),
        secretHash: await digest(text(input.secret, "secret")),
      })
      if (!found) throw new Denied("This device is no longer paired. Pair it again from Cryzo on your computer.")
      const exp = Date.now() + ticketMs
      return { ticket: await sign({ role, host: found.host, ...(found.client ? { client: found.client } : {}), exp }, key), relay, expiresAt: exp }
    }),
  )
  for (const path of ["claim", "ticket"])
    http.route({ path: `/remote/${path}`, method: "OPTIONS", handler: httpAction(async () => new Response(null, { status: 204, headers: cors })) })
}
