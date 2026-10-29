import { ConvexError } from "convex/values"
import { internal } from "./_generated/api"
import { httpAction, type ActionCtx } from "./_generated/server"
import { createAuth } from "./auth"
import { devicePage } from "./lib/device"
import { availableModels, inferenceBody } from "./lib/models"

const nvidia = "https://integrate.api.nvidia.com/v1"
const maxBody = 4_000_000

const security = {
  "Cache-Control": "no-store",
  "X-Content-Type-Options": "nosniff",
  "Referrer-Policy": "no-referrer",
  "Content-Security-Policy": "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; frame-ancestors 'none'; base-uri 'none'",
}

function json(body: unknown, status = 200, headers: Record<string, string> = {}) {
  return new Response(JSON.stringify(body), { status, headers: { ...security, "Content-Type": "application/json", ...headers } })
}

function configured() {
  return Boolean(process.env.SITE_URL && process.env.NVIDIA_API_KEY && process.env.GOOGLE_CLIENT_ID && process.env.GOOGLE_CLIENT_SECRET && process.env.BETTER_AUTH_SECRET)
}

// The desktop holds a Better Auth session token from device authorization; the bearer plugin resolves it here.
async function account(ctx: ActionCtx, request: Request) {
  const authorization = request.headers.get("authorization")
  if (!authorization?.startsWith("Bearer ") || authorization.length > 8192) return null
  const session = await createAuth(ctx).api.getSession({ headers: new Headers({ authorization }) }).catch(() => null)
  return session?.user.id ?? null
}

async function catalog(signal: AbortSignal) {
  const response = await fetch(`${nvidia}/models`, { signal: AbortSignal.any([signal, AbortSignal.timeout(20000)]) })
  if (!response.ok) return null
  return availableModels(await response.json())
}

async function readBody(request: Request) {
  const length = Number(request.headers.get("content-length") ?? 0)
  if (length > maxBody) throw new Error("invalid body size")
  const text = await request.text()
  if (text.length > maxBody) throw new Error("invalid body size")
  try { return JSON.parse(text) as unknown } catch { throw new Error("invalid JSON") }
}

export const health = httpAction(async () => json({ status: "ok", configured: configured() }))

export const device = httpAction(async () => new Response(devicePage, { headers: { ...security, "Content-Type": "text/html; charset=utf-8" } }))

export const models = httpAction(async (ctx, request) => {
  if (!await account(ctx, request)) return json({ error: "Sign in to Cryzo Free" }, 401)
  const data = await catalog(request.signal).catch(() => null)
  if (!data) return json({ error: "NVIDIA catalog unavailable" }, 503)
  return json({ object: "list", data })
})

export const completions = httpAction(async (ctx, request) => {
  try {
    const user = await account(ctx, request)
    if (!user) return json({ error: "Sign in to Cryzo Free" }, 401)
    if (!process.env.NVIDIA_API_KEY) return json({ error: "Free inference is not configured" }, 503)
    const parsed = inferenceBody(await readBody(request))
    const live = await catalog(request.signal).catch(() => null)
    if (!live?.some((model) => model.id === parsed.body.model)) return json({ error: "This free model is temporarily unavailable" }, 503)
    const quota = await ctx.runMutation(internal.quota.reserve, { user, step: parsed.step })
    const controller = new AbortController()
    const signal = AbortSignal.any([request.signal, controller.signal, AbortSignal.timeout(270000)])
    // NVIDIA's free queue can stall before sending headers. Release the user's single slot after
    // a minute instead of holding it until the 270-second deadline.
    const waiting = new AbortController()
    const firstResponse = setTimeout(() => waiting.abort(), 60000)
    let settled = false
    async function finish(retryAfter?: number) {
      if (settled) return
      settled = true
      await ctx.runMutation(internal.quota.finish, { user: user!, step: parsed.step, ...(retryAfter === undefined ? {} : { retryAfter }) })
    }
    const response = await fetch(`${nvidia}/chat/completions`, {
      method: "POST",
      headers: { Authorization: `Bearer ${process.env.NVIDIA_API_KEY}`, "Content-Type": "application/json", Accept: "text/event-stream", "X-Title": "Cryzo", "HTTP-Referer": process.env.SITE_URL ?? "" },
      body: JSON.stringify(parsed.body), signal: AbortSignal.any([signal, waiting.signal]), redirect: "error",
    }).catch(async () => { await finish(); return null }).finally(() => clearTimeout(firstResponse))
    if (!response && waiting.signal.aborted) return json({ error: "This free model is temporarily unavailable" }, 503)
    if (!response) return json({ error: "Could not connect to NVIDIA" }, 502)
    if (!response.ok || !response.body || !response.headers.get("content-type")?.startsWith("text/event-stream")) {
      const seconds = Number(response.headers.get("retry-after"))
      const wait = Number.isFinite(seconds) && seconds > 0 ? Math.min(3600, seconds) : 30
      await finish(response.status === 429 ? wait : undefined)
      if (response.status === 429) return json({ error: "NVIDIA is busy; retry shortly" }, 429, { "Retry-After": String(wait) })
      return json({ error: "NVIDIA inference failed" }, 502)
    }
    const reader = response.body.getReader()
    const stream = new ReadableStream<Uint8Array>({
      async pull(output) {
        try {
          const next = await reader.read()
          if (next.done) { await finish(); output.close(); return }
          output.enqueue(next.value)
        } catch { await finish(); output.error(new Error("NVIDIA stream interrupted")) }
      },
      async cancel() { controller.abort(); await reader.cancel(); await finish() },
    })
    return new Response(stream, { headers: { ...security, "Content-Type": "text/event-stream", "X-Cryzo-Calls-Remaining": String(quota.remaining), "X-Accel-Buffering": "no" } })
  } catch (error) {
    // Never return provider bodies, secrets, prompts, or exception stacks.
    if (error instanceof ConvexError) return json({ error: String(error.data) }, 429, { "Retry-After": "30" })
    const message = error instanceof Error ? error.message : ""
    if (message.startsWith("invalid ") || message === "unsupported free model") return json({ error: message }, 400)
    console.error("Cryzo gateway request failed", { type: error instanceof Error ? error.name : typeof error })
    return json({ error: "Cryzo Free is temporarily unavailable" }, 503)
  }
})
