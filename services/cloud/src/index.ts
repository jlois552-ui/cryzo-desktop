// cryzo-cloud: cloud workspaces for the Cryzo desktop app. Each workspace is a Durable Object that
// owns one Linux container. The desktop engine runs commands, reads and writes files and opens a
// terminal through `/w/<workspace>/...` with a short-lived ticket from the Convex gateway, which
// enforces the daily and monthly caps. Dev servers inside a workspace are previewed on their own
// hostnames, `<port>-<workspace>.preview.<APP_DOMAIN>`.
import { Files } from "@cloudflare/sandbox"
import { DurableObject } from "cloudflare:workers"
import { inside, parseHashes, previewHost, ROOT, SKIPPED, workspaceRoute } from "./route"
import { verify } from "./ticket"

interface Env {
  WORKSPACES: DurableObjectNamespace<Workspace>
  CLOUD_TICKET_KEY: string
  APP_DOMAIN: string
}

const IDLE_MS = 10 * 60_000
const MAX_FILE = 10 * 1024 * 1024
const MAX_OUTPUT = 1024 * 1024
const PREVIEW_COOKIE = "cryzo_preview"

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json", "Cache-Control": "no-store" } })

export default {
  async fetch(request: Request, env: Env) {
    const url = new URL(request.url)
    const preview = previewHost(url.hostname, env.APP_DOMAIN)
    if (preview) return previewRequest(request, env, preview)

    if (url.pathname === "/health") return json({ ok: true })
    const route = workspaceRoute(url.pathname)
    if (!route) return json({ error: "Not found." }, 404)
    const token = request.headers.get("Authorization")?.replace(/^Bearer /, "") ?? url.searchParams.get("ticket") ?? ""
    const ticket = await verify(token, env.CLOUD_TICKET_KEY)
    if (!ticket || ticket.role !== "workspace" || ticket.host !== route.workspace) return json({ error: "Invalid or expired ticket." }, 401)

    const workspace = env.WORKSPACES.getByName(route.workspace)
    if (route.action === "pty") return workspace.fetch(request)
    if (request.method !== "POST") return json({ error: "Use POST." }, 405)
    const input = ((await request.json().catch(() => null)) ?? {}) as Record<string, unknown>
    try {
      switch (route.action) {
        case "exec":
          return json(await workspace.exec(String(input.command ?? ""), path(input.cwd ?? ROOT), Number(input.timeout_ms) || 120_000))
        case "read":
          return json(await workspace.read(path(input.path)))
        case "write":
          return json(await workspace.write(path(input.path), String(input.content ?? "")))
        case "list":
          return json(await workspace.list(path(input.path ?? ROOT)))
        case "delete":
          return json(await workspace.remove(path(input.path)))
        case "hashes":
          return json(await workspace.hashes())
        case "stop":
          return json(await workspace.stop())
      }
    } catch (error) {
      return json({ error: error instanceof Error ? error.message : "The workspace could not do that." }, 400)
    }
  },
}

function path(value: unknown) {
  const resolved = inside(String(value ?? ""))
  if (!resolved) throw new Error("Paths must stay inside the project.")
  return resolved
}

/**
 * A preview needs the workspace's ticket once, as `?t=`; it is then kept in a cookie on that
 * preview's own hostname, so the preview page itself never sees it in its URL again.
 */
async function previewRequest(request: Request, env: Env, preview: { port: number; workspace: string }) {
  const url = new URL(request.url)
  const given = url.searchParams.get("t")
  const cookie = /(?:^|;\s*)cryzo_preview=([^;]+)/.exec(request.headers.get("Cookie") ?? "")?.[1]
  const ticket = await verify(given ?? cookie ?? "", env.CLOUD_TICKET_KEY)
  if (!ticket || ticket.role !== "workspace" || ticket.host !== preview.workspace)
    return new Response("This preview link expired. Open it again from Cryzo.", { status: 401 })
  if (given) {
    url.searchParams.delete("t")
    return new Response(null, {
      status: 302,
      headers: {
        Location: url.toString(),
        "Set-Cookie": `${PREVIEW_COOKIE}=${given}; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age=${Math.max(0, Math.floor((ticket.exp - Date.now()) / 1000))}`,
      },
    })
  }
  return env.WORKSPACES.getByName(preview.workspace).preview(request, preview.port)
}

export class Workspace extends DurableObject<Env> {
  constructor(ctx: DurableObjectState, env: Env) {
    super(ctx, env)
    const container = ctx.container
    if (container?.running) void ctx.blockConcurrencyWhile(() => container.setInactivityTimeout(IDLE_MS))
  }

  /** The running container, started on first use. Vercel-like idle stop: 10 minutes. */
  private async container() {
    const container = this.ctx.container
    if (!container) throw new Error("The workspace container is not configured.")
    if (!container.running) {
      // Internet is on so `npm install` and git work; the token never enters the container.
      container.start({ image: container.images.workspace, enableInternet: true, instance: "standard-1" })
      await container.setInactivityTimeout(IDLE_MS)
    }
    return container
  }

  async exec(command: string, cwd: string, timeoutMs: number) {
    if (!command.trim()) throw new Error("No command.")
    const container = await this.container()
    const process = await container.exec(["bash", "-lc", command], {
      cwd,
      signal: AbortSignal.timeout(Math.min(Math.max(timeoutMs, 1000), 600_000)),
      stdout: "pipe",
      stderr: "pipe",
    })
    const output = await process.output()
    const text = (buffer: ArrayBuffer) => new TextDecoder().decode(buffer.slice(0, MAX_OUTPUT))
    return { code: output.exitCode, stdout: text(output.stdout), stderr: text(output.stderr) }
  }

  async read(path: string) {
    const files = new Files(await this.container())
    const stat = await files.stat(path)
    if (stat.size > BigInt(MAX_FILE)) throw new Error("The file is larger than 10 MB.")
    const bytes = new Uint8Array(await (await files.readFile(path)).arrayBuffer())
    return { content: base64(bytes), size: bytes.length }
  }

  async write(path: string, content: string) {
    const bytes = Uint8Array.from(atob(content), (char) => char.charCodeAt(0))
    if (bytes.length > MAX_FILE) throw new Error("The file is larger than 10 MB.")
    const files = new Files(await this.container())
    const parent = path.slice(0, path.lastIndexOf("/"))
    if (parent) await files.mkdir(parent, { recursive: true })
    await files.writeFile(path, bytes)
    return { size: bytes.length }
  }

  async list(path: string) {
    const entries = await new Files(await this.container()).readDirectory(path)
    return entries.slice(0, 5000).map((entry) => ({ name: entry.name, directory: entry.type === "directory" }))
  }

  async remove(path: string) {
    if (path === ROOT) throw new Error("The project folder itself cannot be deleted.")
    await new Files(await this.container()).remove(path, { recursive: true })
    return { deleted: true }
  }

  /** Every project file's SHA-256, so the desktop syncs only what changed. */
  async hashes() {
    const prune = SKIPPED.map((name) => `-name '${name}'`).join(" -o ")
    const result = await this.exec(
      `find . \\( ${prune} \\) -prune -o -type f -print0 | head -z -n 5000 | xargs -0 -r sha256sum`,
      ROOT,
      120_000,
    )
    if (result.code !== 0) throw new Error(result.stderr || "The workspace could not list its files.")
    return parseHashes(result.stdout)
  }

  async stop() {
    if (this.ctx.container?.running) await this.ctx.container.destroy()
    return { stopped: true }
  }

  async preview(request: Request, port: number) {
    const container = await this.container()
    try {
      return await container.getTcpPort(port).fetch(request)
    } catch {
      return new Response(`Nothing is running on port ${port} yet. Start the dev server on 0.0.0.0:${port}.`, { status: 502 })
    }
  }

  /**
   * The terminal: a tmux session that survives the socket closing. Binary frames are keystrokes,
   * text frames are resizes as `{"cols":..,"rows":..}`.
   */
  override async fetch(request: Request) {
    if (request.headers.get("Upgrade")?.toLowerCase() !== "websocket") return new Response("Expected a WebSocket upgrade.", { status: 426 })
    const url = new URL(request.url)
    const container = await this.container()
    const abort = new AbortController()
    const tmux = await container.exec(["tmux", "new-session", "-A", "-s", "main"], {
      pty: { cols: Number(url.searchParams.get("cols")) || 100, rows: Number(url.searchParams.get("rows")) || 30 },
      env: { TERM: "xterm-256color" },
      cwd: ROOT,
      signal: abort.signal,
    })
    const [client, server] = Object.values(new WebSocketPair()) as [WebSocket, WebSocket]
    server.binaryType = "arraybuffer"
    // Not hibernation: it would drop the tmux process handle.
    server.accept()
    const stdin = tmux.stdin!.getWriter()
    server.addEventListener("message", (event) => {
      if (typeof event.data === "string") {
        try {
          const size = JSON.parse(event.data) as { cols: number; rows: number }
          tmux.resize(size.cols, size.rows)
        } catch {
          // Not a resize.
        }
        return
      }
      stdin.write(new Uint8Array(event.data as ArrayBuffer)).catch(() => undefined)
    })
    let exited = false
    void tmux.exitCode.then(
      () => (exited = true),
      () => (exited = true),
    )
    server.addEventListener("close", () => {
      if (!exited) abort.abort()
    })
    void (async () => {
      for await (const chunk of tmux.stdout as ReadableStream<Uint8Array>) server.send(chunk)
      server.close(1000, "Terminal closed")
    })().catch(() => server.close(1011, "Terminal output failed"))
    return new Response(null, { status: 101, webSocket: client })
  }
}

function base64(bytes: Uint8Array) {
  let text = ""
  for (let index = 0; index < bytes.length; index += 0x8000) text += String.fromCharCode(...bytes.subarray(index, index + 0x8000))
  return btoa(text)
}
