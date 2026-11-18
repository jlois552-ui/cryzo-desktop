// Publishing HTTP API for the desktop engine, with the user's Better Auth bearer token.
import type { HttpRouter } from "convex/server"
import { internal } from "../_generated/api"
import { httpAction, type ActionCtx } from "../_generated/server"
import { account } from "../gateway"
import { readAssetRouting, readMigrations, readModules, readStorage } from "../lib/apps"

const headers = { "Content-Type": "application/json", "Cache-Control": "no-store", "X-Content-Type-Options": "nosniff" }
const reply = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers })

function route(handler: (ctx: ActionCtx, user: string, input: Record<string, unknown>) => Promise<unknown>) {
  return httpAction(async (ctx, request) => {
    const user = await account(ctx, request)
    if (!user) return reply({ error: "Sign in to Cryzo to publish apps.", code: "sign_in" }, 401)
    const input =
      request.method === "GET"
        ? Object.fromEntries(new URL(request.url).searchParams)
        : ((await request.json().catch(() => null)) as Record<string, unknown> | null)
    if (!input) return reply({ error: "Invalid request." }, 400)
    try {
      return reply({ data: (await handler(ctx, user, input)) ?? null })
    } catch (error) {
      const message = error instanceof Error ? error.message : "Request failed."
      return reply({ error: message.replace(/^\[.*?\]\s*/, "").replace(/^Uncaught Error:\s*/, "").split("\n")[0] }, 400)
    }
  })
}

const text = (value: unknown) => (typeof value === "string" ? value : "")

export function appRoutes(http: HttpRouter) {
  http.route({ path: "/apps", method: "GET", handler: route((ctx, user) => ctx.runQuery(internal.apps.store.list, { user })) })
  http.route({
    path: "/apps/publish/begin",
    method: "POST",
    handler: route((ctx, user, input) =>
      ctx.runAction(internal.apps.cloudflare.begin, {
        user,
        slug: text(input.slug),
        manifest: input.manifest,
        storage: readStorage(input.storage),
      }),
    ),
  })
  http.route({
    path: "/apps/publish/finish",
    method: "POST",
    handler: route((ctx, user, input) =>
      ctx.runAction(internal.apps.cloudflare.finish, {
        user,
        slug: text(input.slug),
        jwt: text(input.jwt),
        modules: readModules(input.modules),
        routing: readAssetRouting(input.assets),
        storage: readStorage(input.storage),
        migrations: readMigrations(input.migrations),
      }),
    ),
  })
  http.route({
    path: "/apps/migrations",
    method: "POST",
    handler: route(async (ctx, user, input) => (await ctx.runQuery(internal.apps.store.get, { user, slug: text(input.slug) }))?.migrations ?? []),
  })
  http.route({
    path: "/apps/data/export",
    method: "POST",
    handler: route((ctx, user, input) => ctx.runAction(internal.apps.cloudflare.exportData, { user, slug: text(input.slug) })),
  })
  http.route({
    path: "/apps/secrets",
    method: "POST",
    handler: route((ctx, user, input) => ctx.runAction(internal.apps.cloudflare.secrets, { user, slug: text(input.slug) })),
  })
  http.route({
    path: "/apps/secrets/set",
    method: "POST",
    handler: route((ctx, user, input) =>
      ctx.runAction(internal.apps.cloudflare.setSecret, { user, slug: text(input.slug), name: text(input.name), value: text(input.value) }),
    ),
  })
  http.route({
    path: "/apps/secrets/delete",
    method: "POST",
    handler: route((ctx, user, input) =>
      ctx.runAction(internal.apps.cloudflare.deleteSecret, { user, slug: text(input.slug), name: text(input.name) }),
    ),
  })
  http.route({
    path: "/apps/unpublish",
    method: "POST",
    handler: route((ctx, user, input) => ctx.runAction(internal.apps.cloudflare.unpublish, { user, slug: text(input.slug) })),
  })
}
