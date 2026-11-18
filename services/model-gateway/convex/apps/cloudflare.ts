// Publishing to Cloudflare. The desktop sends a manifest of the build output, uploads the files
// straight to Cloudflare with the short-lived upload token from `begin`, then calls `finish`,
// which deploys the Worker, points `<slug>.<app domain>` at it and sets its sign-in secret.
// The Cloudflare API token stays here; the desktop and the app never see it.
import { v } from "convex/values"
import { internal } from "../_generated/api"
import { internalAction, type ActionCtx } from "../_generated/server"
import { bucketName, managedSecret, manifestProblem, scriptName, secretProblem, slugProblem, STATIC_MODULE, type Manifest } from "../lib/apps"

const API = "https://api.cloudflare.com/client/v4"

function config() {
  const account = process.env.CLOUDFLARE_ACCOUNT_ID?.trim()
  const token = process.env.CLOUDFLARE_API_TOKEN?.trim()
  const zone = process.env.CLOUDFLARE_ZONE_ID?.trim()
  const domain = process.env.CLOUD_APP_DOMAIN?.trim()
  if (!account || !token || !zone || !domain) throw new Error("Publishing is not set up on the Cryzo cloud yet.")
  return { account, token, zone, domain }
}

async function cloudflare(path: string, init: RequestInit = {}) {
  const settings = config()
  const response = await fetch(`${API}/accounts/${settings.account}${path}`, {
    ...init,
    headers: { Authorization: `Bearer ${settings.token}`, ...init.headers },
    signal: AbortSignal.timeout(60_000),
  })
  // Deleting something that is already gone has done what was asked.
  if (response.status === 404 && init.method === "DELETE") return {}
  const body = (await response.json().catch(() => null)) as { success?: boolean; result?: unknown; errors?: { message?: string }[] } | null
  if (!response.ok || !body?.success) throw new Error(`Cloudflare refused the request: ${body?.errors?.[0]?.message ?? response.status}`)
  return body.result as Record<string, unknown>
}

const json = (body: unknown): RequestInit => ({ body: JSON.stringify(body), headers: { "Content-Type": "application/json" } })

type Begun = { jwt: string; buckets: string[][]; upload: string; warning?: string }

const storage = v.object({ database: v.string(), files: v.optional(v.string()) })

export const begin = internalAction({
  // `storage` is checked by `readStorage` at the HTTP boundary.
  args: { user: v.string(), slug: v.string(), manifest: v.any(), storage },
  handler: async (ctx, args): Promise<Begun> => {
    const problem = slugProblem(args.slug) ?? manifestProblem(args.manifest)
    if (problem) throw new Error(problem)
    const settings = config()
    const script = scriptName(args.slug)
    const app = await ctx.runMutation(internal.apps.store.claim, { user: args.user, slug: args.slug, script })
    // Every app gets its own database on first publish, so sign-in and data work without setup.
    if (!app.database) {
      const created = await cloudflare("/d1/database", { method: "POST", ...json({ name: script }) })
      await ctx.runMutation(internal.apps.store.update, { user: args.user, slug: args.slug, database: String(created.uuid) })
    }
    // A file bucket only when the app's settings ask for one, so plain apps don't need R2. If R2
    // can't make one, the app still publishes without file storage and the next publish retries.
    let warning: string | undefined
    if (args.storage.files && !app.bucket) {
      const created = await cloudflare("/r2/buckets", { method: "POST", ...json({ name: bucketName(args.slug) }) }).then(
        () => true,
        // Creating it again after a lost response is fine; the bucket is already there.
        (error: unknown) => error instanceof Error && /already exists/i.test(error.message),
      )
      if (created) await ctx.runMutation(internal.apps.store.update, { user: args.user, slug: args.slug, bucket: bucketName(args.slug) })
      else warning = "Published without file storage, which isn't available yet. File uploads in this app will say they aren't set up."
    }
    const session = await cloudflare(`/workers/scripts/${script}/assets-upload-session`, {
      method: "POST",
      ...json({ manifest: args.manifest as Manifest }),
    })
    return {
      jwt: String(session.jwt),
      buckets: (session.buckets as string[][] | undefined) ?? [],
      upload: `${API}/accounts/${settings.account}/workers/assets/upload?base64=true`,
      ...(warning && { warning }),
    }
  },
})

const routing = v.object({
  not_found_handling: v.union(v.literal("single-page-application"), v.literal("404-page"), v.literal("none")),
  html_handling: v.optional(
    v.union(v.literal("auto-trailing-slash"), v.literal("force-trailing-slash"), v.literal("drop-trailing-slash"), v.literal("none")),
  ),
  run_worker_first: v.optional(v.union(v.boolean(), v.array(v.string()))),
})

export const finish = internalAction({
  args: {
    user: v.string(),
    slug: v.string(),
    jwt: v.string(),
    // Checked by `readModules` and `readAssetRouting` at the HTTP boundary.
    modules: v.array(v.object({ name: v.string(), content: v.string() })),
    routing,
    storage,
    migrations: v.array(v.object({ name: v.string(), hash: v.string() })),
  },
  handler: async (ctx, args): Promise<{ url: string }> => {
    const settings = config()
    const app = await ctx.runQuery(internal.apps.store.get, { user: args.user, slug: args.slug })
    if (!app?.database) throw new Error("Start publishing again; this app's upload was not found.")
    const hostname = `${args.slug}.${settings.domain}`
    const url = `https://${hostname}`
    const modules = args.modules.length > 0 ? args.modules : [{ name: "index.js", content: STATIC_MODULE }]
    const form = new FormData()
    form.append(
      "metadata",
      new Blob(
        [
          JSON.stringify({
            main_module: modules[0]!.name,
            compatibility_date: "2026-11-01",
            compatibility_flags: ["nodejs_compat"],
            assets: { jwt: args.jwt, config: args.routing },
            bindings: [
              { type: "assets", name: "ASSETS" },
              { type: "d1", name: args.storage.database, id: app.database },
              ...(app.bucket && args.storage.files ? [{ type: "r2_bucket", name: args.storage.files, bucket_name: app.bucket }] : []),
              { type: "plain_text", name: "APP_URL", text: url },
            ],
            // Secrets are set once below and must survive every later deploy.
            keep_bindings: ["secret_text"],
          }),
        ],
        { type: "application/json" },
      ),
    )
    for (const module of modules)
      form.append(module.name, new Blob([module.content], { type: "application/javascript+module" }), module.name)
    await cloudflare(`/workers/scripts/${app.script}`, { method: "PUT", body: form })
    // Only the app's own address serves it, not a second workers.dev one.
    await cloudflare(`/workers/scripts/${app.script}/subdomain`, { method: "POST", ...json({ enabled: false, previews_enabled: false }) })
    await cloudflare("/workers/domains", {
      method: "PUT",
      ...json({ environment: "production", hostname, service: app.script, zone_id: settings.zone }),
    })
    if (!app.secret) {
      const bytes = crypto.getRandomValues(new Uint8Array(32))
      await cloudflare(`/workers/scripts/${app.script}/secrets`, {
        method: "PUT",
        ...json({ name: "BETTER_AUTH_SECRET", text: Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join(""), type: "secret_text" }),
      })
    }
    // The migrations now run on the live database the first time the new code serves a request.
    await ctx.runMutation(internal.apps.store.update, {
      user: args.user,
      slug: args.slug,
      secret: true,
      url,
      publishedAt: Date.now(),
      migrations: args.migrations,
    })
    return { url }
  },
})

/** Takes the app offline and deletes its database, freeing the user's app slot. */
export const unpublish = internalAction({
  args: { user: v.string(), slug: v.string() },
  handler: async (ctx, args): Promise<void> => {
    const app = await ctx.runQuery(internal.apps.store.get, { user: args.user, slug: args.slug })
    if (!app) return
    // `force` also removes the app's custom domain.
    await cloudflare(`/workers/scripts/${app.script}?force=true`, { method: "DELETE" })
    if (app.database) await cloudflare(`/d1/database/${app.database}`, { method: "DELETE" })
    if (app.bucket) await deleteBucket(app.bucket)
    await ctx.runMutation(internal.apps.store.remove, args)
  },
})

// App secrets: API keys and the like, stored only as Cloudflare Worker secrets. Values go in and
// are never read back, so neither the desktop, the agent nor this gateway can show them again.

async function published(ctx: ActionCtx, user: string, slug: string) {
  const app = await ctx.runQuery(internal.apps.store.get, { user, slug })
  if (!app?.url) throw new Error("Publish the app first, then add its secrets.")
  return app
}

/** The names of the app's own secrets (not the ones Cryzo manages). */
export const secrets = internalAction({
  args: { user: v.string(), slug: v.string() },
  handler: async (ctx, args): Promise<string[]> => {
    const app = await published(ctx, args.user, args.slug)
    const list = (await cloudflare(`/workers/scripts/${app.script}/secrets`)) as unknown as { name: string }[]
    return list
      .map((secret) => secret.name)
      .filter((name) => !managedSecret(name))
      .sort()
  },
})

/** Adds or replaces a secret. The app restarts with it right away. */
export const setSecret = internalAction({
  args: { user: v.string(), slug: v.string(), name: v.string(), value: v.string() },
  handler: async (ctx, args): Promise<void> => {
    const problem = secretProblem(args.name, args.value)
    if (problem) throw new Error(problem)
    const app = await published(ctx, args.user, args.slug)
    await cloudflare(`/workers/scripts/${app.script}/secrets`, {
      method: "PUT",
      ...json({ name: args.name, text: args.value, type: "secret_text" }),
    })
  },
})

export const deleteSecret = internalAction({
  args: { user: v.string(), slug: v.string(), name: v.string() },
  handler: async (ctx, args): Promise<void> => {
    const problem = secretProblem(args.name)
    if (problem) throw new Error(problem)
    const app = await published(ctx, args.user, args.slug)
    await cloudflare(`/workers/scripts/${app.script}/secrets/${args.name}`, { method: "DELETE" })
  },
})

/** R2 deletes only empty buckets, so the app's files go first. */
async function deleteBucket(name: string) {
  for (;;) {
    const page = await list(`/r2/buckets/${name}/objects?per_page=1000`)
    if (page === undefined) return
    const keys = page.map((object) => object.key)
    if (keys.length === 0) break
    for (let index = 0; index < keys.length; index += 20)
      await Promise.all(
        keys.slice(index, index + 20).map((key) => cloudflare(`/r2/buckets/${name}/objects/${encodeURIComponent(key)}`, { method: "DELETE" })),
      )
  }
  await cloudflare(`/r2/buckets/${name}`, { method: "DELETE" })
}

/** A Cloudflare list result, or undefined when the parent is already gone. */
async function list(path: string) {
  const settings = config()
  const response = await fetch(`${API}/accounts/${settings.account}${path}`, {
    headers: { Authorization: `Bearer ${settings.token}` },
    signal: AbortSignal.timeout(60_000),
  })
  if (response.status === 404) return undefined
  const body = (await response.json().catch(() => null)) as { success?: boolean; result?: { key: string }[]; errors?: { message?: string }[] } | null
  if (!response.ok || !body?.success) throw new Error(`Cloudflare refused the request: ${body?.errors?.[0]?.message ?? response.status}`)
  return body.result ?? []
}

/**
 * A download link for the live database as SQL. Cloudflare cancels an export that isn't polled,
 * so this keeps asking until the file is ready; the link works for an hour.
 */
export const exportData = internalAction({
  args: { user: v.string(), slug: v.string() },
  handler: async (ctx, args): Promise<{ url: string; filename: string }> => {
    const app = await published(ctx, args.user, args.slug)
    if (!app.database) throw new Error("This app has no database yet.")
    let bookmark: string | undefined
    for (let attempt = 0; attempt < 240; attempt++) {
      const result = (await cloudflare(`/d1/database/${app.database}/export`, {
        method: "POST",
        ...json({ output_format: "polling", ...(bookmark && { current_bookmark: bookmark }) }),
      })) as { at_bookmark?: string; status?: string; error?: string; result?: { filename?: string; signed_url?: string } }
      if (result.error) throw new Error(`Cloudflare couldn't export the data: ${result.error}`)
      if (result.result?.signed_url) return { url: result.result.signed_url, filename: result.result.filename ?? `${args.slug}.sql` }
      bookmark = result.at_bookmark ?? bookmark
      await new Promise((resolve) => setTimeout(resolve, 1000))
    }
    throw new Error("Exporting the data took too long. Try again.")
  },
})
