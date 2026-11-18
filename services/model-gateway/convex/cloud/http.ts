// Cloud workspace HTTP API for the desktop engine, with the user's Better Auth bearer token.
import type { HttpRouter } from "convex/server"
import { internal } from "../_generated/api"
import { httpAction, type ActionCtx } from "../_generated/server"
import { account } from "../gateway"

const headers = { "Content-Type": "application/json", "Cache-Control": "no-store", "X-Content-Type-Options": "nosniff" }
const reply = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers })

function route(handler: (ctx: ActionCtx, user: string, project: string) => Promise<unknown>) {
  return httpAction(async (ctx, request) => {
    const user = await account(ctx, request)
    if (!user) return reply({ error: "Sign in to Cryzo to use cloud workspaces.", code: "sign_in" }, 401)
    const input =
      request.method === "GET"
        ? Object.fromEntries(new URL(request.url).searchParams)
        : ((await request.json().catch(() => null)) as Record<string, unknown> | null)
    const project = input?.project
    if (typeof project !== "string" || !project.trim() || project.length > 300) return reply({ error: "Invalid project." }, 400)
    try {
      return reply({ data: (await handler(ctx, user, project.trim())) ?? null })
    } catch (error) {
      const message = error instanceof Error ? error.message : "Request failed."
      return reply({ error: message.replace(/^\[.*?\]\s*/, "").replace(/^Uncaught Error:\s*/, "").split("\n")[0] }, 400)
    }
  })
}

export function cloudRoutes(http: HttpRouter) {
  http.route({ path: "/cloud/start", method: "POST", handler: route((ctx, user, project) => ctx.runAction(internal.cloud.worker.start, { user, project })) })
  http.route({ path: "/cloud/stop", method: "POST", handler: route((ctx, user, project) => ctx.runAction(internal.cloud.worker.stop, { user, project })) })
  // The desktop calls this every minute while a cloud project is open; usage is counted here.
  http.route({
    path: "/cloud/heartbeat",
    method: "POST",
    handler: route(async (ctx, user, project) => {
      const state = await ctx.runMutation(internal.cloud.store.heartbeat, { user, project, active: true })
      if (state.running && !state.allowed.ok) {
        await ctx.runAction(internal.cloud.worker.stop, { user, project })
        return { running: false, reason: state.allowed.reason }
      }
      return { running: state.running, minutesLeft: state.running && state.allowed.ok ? Math.floor(state.allowed.minutes) : 0 }
    }),
  })
  http.route({ path: "/cloud/status", method: "GET", handler: route((ctx, user, project) => ctx.runQuery(internal.cloud.store.get, { user, project })) })
}
