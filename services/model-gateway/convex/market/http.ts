// Marketing HTTP API for the desktop engine. The engine forwards the user's Better Auth
// bearer token; every route resolves it once and passes the user to internal functions.
import type { HttpRouter } from "convex/server"
import { ConvexError } from "convex/values"
import { internal } from "../_generated/api"
import type { Id } from "../_generated/dataModel"
import { httpAction, type ActionCtx } from "../_generated/server"
import { account } from "../gateway"
import { challenge, random } from "../lib/crypto"
import { channels, isChannel, labels, type Channel } from "../lib/market"
import { client, pending, providers, redirect } from "../lib/providers"

const headers = { "Content-Type": "application/json", "Cache-Control": "no-store", "X-Content-Type-Options": "nosniff" }
const reply = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers })

type Input = Record<string, any>
type Handler = (ctx: ActionCtx, user: string, input: Input) => Promise<unknown>

function route(handler: Handler) {
  return httpAction(async (ctx, request) => {
    const user = await account(ctx, request)
    if (!user) return reply({ error: "Sign in to Cryzo to use Marketing.", code: "sign_in" }, 401)
    const input: Input =
      request.method === "GET"
        ? Object.fromEntries(new URL(request.url).searchParams)
        : await request.json().catch(() => null)
    if (!input || typeof input !== "object" || Array.isArray(input)) return reply({ error: "Invalid request body." }, 400)
    try {
      return reply({ data: (await handler(ctx, user, input)) ?? null })
    } catch (error) {
      // Mutations throw plain Errors with user-facing messages; Convex wraps them for HTTP actions.
      const message = error instanceof ConvexError ? String(error.data) : error instanceof Error ? error.message : "Request failed."
      return reply({ error: message.replace(/^\[.*?\]\s*/, "").replace(/^Uncaught Error:\s*/, "").split("\n")[0] }, 400)
    }
  })
}

const text = (value: unknown, name: string) => {
  if (typeof value !== "string" || !value || value.length > 300) throw new Error(`Invalid ${name}.`)
  return value
}
const time = (value: unknown, name: string) => {
  const parsed = Number(value)
  if (!Number.isFinite(parsed)) throw new Error(`Invalid ${name}.`)
  return parsed
}

// The browser lands here after each network's sign-in. There is no Cryzo bearer token on this
// request: the single-use state row identifies the user who started the sign-in.
function callback(channel: Channel) {
  return httpAction(async (ctx, request) => {
    const params = new URL(request.url).searchParams
    const started = params.get("state") ? await ctx.runMutation(internal.market.store.takeOauth, { state: params.get("state")!, channel }) : null
    if (!started) return page(false, "This sign-in link expired or was already used. Start again from Marketing in Cryzo.")
    const code = params.get("code")
    if (params.get("error") || !code) return page(false, `${labels[channel]} sign-in was cancelled.`)
    try {
      await ctx.runAction(internal.market.worker.finishConnect, { user: started.user, channel, code, ...(started.verifier ? { verifier: started.verifier } : {}) })
      return page(true, `Your ${labels[channel]} account is connected. Return to Cryzo.`)
    } catch (error) {
      const message = error instanceof Error ? error.message.replace(/^\[.*?\]\s*/, "").replace(/^Uncaught Error:\s*/, "").split("\n")[0] : ""
      return page(false, `${labels[channel]} could not be connected. ${message}`.trim())
    }
  })
}

function page(ok: boolean, message: string) {
  const escaped = message.replace(/[&<>"']/g, (char) => `&#${char.charCodeAt(0)};`)
  const title = ok ? "Channel connected" : "Channel not connected"
  return new Response(
    `<!doctype html><meta charset=utf-8><meta name=viewport content="width=device-width"><title>${title}</title><style>body{font-family:system-ui,sans-serif;display:grid;place-items:center;min-height:100vh;margin:0;padding:24px;box-sizing:border-box}main{max-width:420px}</style><main><h1>${title}</h1><p>${escaped}</p></main>`,
    {
      status: ok ? 200 : 400,
      headers: { "Content-Type": "text/html; charset=utf-8", "Cache-Control": "no-store", "Referrer-Policy": "no-referrer", "X-Content-Type-Options": "nosniff" },
    },
  )
}

export function marketRoutes(http: HttpRouter) {
  const get = (path: string, handler: Handler) => http.route({ path: `/market/${path}`, method: "GET", handler: route(handler) })
  const post = (path: string, handler: Handler) => http.route({ path: `/market/${path}`, method: "POST", handler: route(handler) })
  for (const channel of channels) http.route({ path: `/market/oauth/${channel}/callback`, method: "GET", handler: callback(channel) })

  // Which networks are set up on this deployment, and which still wait on platform approval.
  get("providers", async () =>
    channels.map((channel) => ({ channel, configured: Boolean(client(channel)), pending: pending(channel), pages: channel === "linkedin" && !pending("linkedin-page") })),
  )
  get("accounts", (ctx, user) => ctx.runQuery(internal.market.store.listAccounts, { user }))
  // Returns the network's sign-in URL; the desktop opens it in the browser and polls `accounts`.
  post("accounts/connect", async (ctx, user, input) => {
    if (!isChannel(input.channel)) throw new Error("Choose a supported channel.")
    const app = client(input.channel)
    if (!app) throw new Error(`${labels[input.channel]} is not set up on the Cryzo cloud yet.`)
    const provider = providers[input.channel]
    const state = random()
    const verifier = provider.pkce ? random(48) : undefined
    await ctx.runMutation(internal.market.store.startOauth, { user, channel: input.channel, state, ...(verifier ? { verifier } : {}) })
    return provider.authorize({ client: app, redirect: redirect(input.channel), state, ...(verifier ? { challenge: await challenge(verifier) } : {}) })
  })
  post("accounts/update", (ctx, user, input) =>
    ctx.runMutation(internal.market.store.updateAccount, {
      user,
      connectedAccountId: text(input.connectedAccountId, "channel"),
      ...(input.name !== undefined ? { name: String(input.name) } : {}),
      ...(input.groupId !== undefined ? { groupId: input.groupId === null ? null : (text(input.groupId, "group") as Id<"marketGroups">) } : {}),
    }),
  )
  post("accounts/disconnect", (ctx, user, input) =>
    ctx.runMutation(internal.market.store.removeAccount, { user, connectedAccountId: text(input.connectedAccountId, "channel") }),
  )
  get("accounts/targets", (ctx, user, input) =>
    ctx.runAction(internal.market.worker.targets, { user, connectedAccountId: text(input.connectedAccountId, "channel") }),
  )
  post("accounts/target", (ctx, user, input) =>
    ctx.runAction(internal.market.worker.chooseTarget, {
      user,
      connectedAccountId: text(input.connectedAccountId, "channel"),
      targetId: text(input.targetId, "page"),
    }),
  )

  post("groups/save", (ctx, user, input) =>
    ctx.runMutation(internal.market.store.saveGroup, {
      user,
      name: String(input.name ?? ""),
      color: String(input.color ?? ""),
      ...(input.groupId ? { groupId: text(input.groupId, "group") as Id<"marketGroups"> } : {}),
    }),
  )
  post("groups/delete", (ctx, user, input) =>
    ctx.runMutation(internal.market.store.deleteGroup, { user, groupId: text(input.groupId, "group") as Id<"marketGroups"> }),
  )

  get("posts", (ctx, user, input) =>
    ctx.runQuery(internal.market.store.listPosts, { user, from: time(input.from, "start"), to: time(input.to, "end") }),
  )
  post("posts/create", (ctx, user, input) =>
    ctx.runMutation(internal.market.store.createPost, {
      user,
      requestKey: text(input.requestKey, "request"),
      content: String(input.content ?? ""),
      targets: input.targets,
      scheduledFor: time(input.scheduledFor, "date"),
      status: input.status === "scheduled" ? "scheduled" : "draft",
      mediaIds: input.mediaIds ?? [],
      ...(input.options ? { options: input.options } : {}),
      ...(input.groupId ? { groupId: input.groupId } : {}),
    }),
  )
  post("posts/update", (ctx, user, input) => {
    const { postId, ...fields } = input
    return ctx.runMutation(internal.market.store.updatePost, { user, postId: text(postId, "post") as Id<"marketPosts">, ...fields })
  })
  post("posts/delete", (ctx, user, input) =>
    ctx.runMutation(internal.market.store.deletePost, { user, postId: text(input.postId, "post") as Id<"marketPosts"> }),
  )
  post("posts/publish", (ctx, user, input) =>
    ctx.runMutation(internal.market.store.publishNow, { user, postId: text(input.postId, "post") as Id<"marketPosts"> }),
  )
  post("posts/retry", (ctx, user, input) =>
    ctx.runMutation(internal.market.store.retryDelivery, { user, deliveryId: text(input.deliveryId, "delivery") as Id<"marketDeliveries"> }),
  )

  post("media/upload-url", (ctx) => ctx.runMutation(internal.market.store.uploadUrl, {}))
  post("media/register", (ctx, user, input) =>
    ctx.runMutation(internal.market.store.registerMedia, {
      user,
      storageId: text(input.storageId, "upload") as Id<"_storage">,
      name: String(input.name ?? ""),
    }),
  )
  get("media", (ctx, user) => ctx.runQuery(internal.market.store.listMedia, { user }))
  post("media/delete", (ctx, user, input) =>
    ctx.runMutation(internal.market.store.deleteMedia, { user, storageId: text(input.storageId, "media") as Id<"_storage"> }),
  )

  get("analytics", (ctx, user, input) =>
    ctx.runAction(internal.market.worker.analytics, { user, days: Number(input.days ?? 30), refresh: input.refresh === "1" }),
  )
}
