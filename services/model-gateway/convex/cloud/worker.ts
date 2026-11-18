// Cloud workspaces run on the cryzo-cloud Cloudflare Worker (services/cloud). Starting one is
// only a ticket: the Worker starts the container on first use. The ticket is returned to the
// desktop once and never stored; it lasts until the user's remaining allowance, at most an hour.
import { v } from "convex/values"
import { internal } from "../_generated/api"
import { internalAction } from "../_generated/server"
import { sandboxName } from "../lib/cloud"
import { sign } from "../lib/ticket"

const IDLE_MS = 10 * 60_000
const CHECK_MS = 5 * 60_000
const TICKET_MS = 60 * 60_000

function config() {
  const url = process.env.CLOUD_URL?.trim().replace(/\/+$/, "")
  const key = process.env.CLOUD_TICKET_KEY?.trim()
  const domain = process.env.CLOUD_APP_DOMAIN?.trim()
  if (!url || !key || !domain) throw new Error("Cloud workspaces are not set up on the Cryzo cloud yet.")
  return { url, key, domain }
}

const workspace = { user: v.string(), project: v.string() }

type Started = { url: string; token: string; preview: string; minutes: number }

export const start = internalAction({
  args: workspace,
  handler: async (ctx, args): Promise<Started> => {
    const { url, key, domain } = config()
    const reserved: { sandbox: string; minutes: number } = await ctx.runMutation(internal.cloud.store.reserve, args)
    const exp = Date.now() + Math.min(Math.floor(reserved.minutes) * 60_000, TICKET_MS)
    const token = await sign({ role: "workspace", host: reserved.sandbox, exp }, key)
    const endpoint = `${url}/w/${reserved.sandbox}`
    // `{port}` is filled in by the desktop for whichever dev server is running.
    const preview = `https://{port}-${reserved.sandbox}.preview.${domain}/`
    await ctx.runMutation(internal.cloud.store.started, { ...args, url: endpoint, previews: { template: preview } })
    await ctx.scheduler.runAfter(IDLE_MS, internal.cloud.worker.check, args)
    return { url: endpoint, token, preview, minutes: Math.floor(reserved.minutes) }
  },
})

export const stop = internalAction({
  args: workspace,
  handler: async (ctx, args): Promise<void> => {
    const { url, key } = config()
    const name = await sandboxName(args.user, args.project)
    const token = await sign({ role: "workspace", host: name, exp: Date.now() + 60_000 }, key)
    // A workspace that is already gone is stopped as far as the user is concerned.
    await fetch(`${url}/w/${name}/stop`, { method: "POST", headers: { Authorization: `Bearer ${token}` }, signal: AbortSignal.timeout(10_000) }).catch(
      () => undefined,
    )
    await ctx.runMutation(internal.cloud.store.stopped, args)
  },
})

/** Stops an idle or over-allowance workspace, otherwise checks again later. */
export const check = internalAction({
  args: workspace,
  handler: async (ctx, args): Promise<void> => {
    const state = await ctx.runMutation(internal.cloud.store.heartbeat, { ...args, active: false })
    if (!state.running) return
    if (!state.allowed.ok || Date.now() - state.lastActiveAt > IDLE_MS) {
      await ctx.runAction(internal.cloud.worker.stop, args)
      return
    }
    await ctx.scheduler.runAfter(CHECK_MS, internal.cloud.worker.check, args)
  },
})
