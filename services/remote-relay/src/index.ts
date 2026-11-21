// Cryzo remote control relay. The desktop engine connects out to it, so the user's computer never
// opens a port. One Durable Object per computer holds the engine's socket and its clients' sockets.
// Tickets come from the Convex gateway and are checked here without calling it (see ticket.ts).
import { DurableObject } from "cloudflare:workers"
import { fromClient, fromHost, newest } from "./route"
import { verify } from "./ticket"

interface Env {
  HOSTS: DurableObjectNamespace<Host>
  REMOTE_TICKET_KEY: string
}

export default {
  async fetch(request: Request, env: Env) {
    const url = new URL(request.url)
    if (url.pathname === "/health") return Response.json({ ok: true })

    if (url.pathname === "/connect") {
      if (request.headers.get("Upgrade")?.toLowerCase() !== "websocket") return new Response("Expected a WebSocket upgrade.", { status: 426 })
      const ticket = await verify(url.searchParams.get("ticket") ?? "", env.REMOTE_TICKET_KEY)
      if (!ticket || (ticket.role !== "host" && ticket.role !== "client") || (ticket.role === "client" && !ticket.client))
        return new Response("Invalid or expired ticket.", { status: 401 })
      const stub = env.HOSTS.get(env.HOSTS.idFromName(ticket.host))
      return stub.fetch(
        new Request("https://relay/connect", {
          headers: { Upgrade: "websocket", "X-Role": ticket.role, "X-Client": ticket.client ?? "" },
        }),
      )
    }

    // The gateway drops a revoked device's open connection.
    if (url.pathname === "/kick" && request.method === "POST") {
      const body = (await request.json().catch(() => null)) as { ticket?: unknown } | null
      const ticket = typeof body?.ticket === "string" ? await verify(body.ticket, env.REMOTE_TICKET_KEY) : null
      if (!ticket || ticket.role !== "kick" || !ticket.client) return new Response("Invalid or expired ticket.", { status: 401 })
      await env.HOSTS.get(env.HOSTS.idFromName(ticket.host)).kick(ticket.client)
      return new Response(null, { status: 204 })
    }

    return new Response("Not found.", { status: 404 })
  },
}

export class Host extends DurableObject<Env> {
  constructor(ctx: DurableObjectState, env: Env) {
    super(ctx, env)
    // Both ends send "ping" to notice a dead connection; answering here never wakes this object.
    ctx.setWebSocketAutoResponse(new WebSocketRequestResponsePair("ping", "pong"))
  }

  override async fetch(request: Request) {
    const role = request.headers.get("X-Role")
    const client = request.headers.get("X-Client") ?? ""
    const [outside, inside] = Object.values(new WebSocketPair()) as [WebSocket, WebSocket]

    if (role === "host") {
      // One engine per computer: a reconnect replaces a stale socket.
      for (const old of this.ctx.getWebSockets("host")) old.close(4000, "Replaced by a newer connection.")
      this.ctx.acceptWebSocket(inside, ["host", `h:${Date.now()}`])
      this.toClients({ t: "online" })
      inside.send(JSON.stringify({ t: "clients", n: this.ctx.getWebSockets("client").length }))
    } else {
      this.ctx.acceptWebSocket(inside, ["client", `c:${client}`])
      inside.send(JSON.stringify({ t: this.ctx.getWebSockets("host").length ? "online" : "offline" }))
      this.toHost({ t: "clients", n: this.ctx.getWebSockets("client").length })
    }
    return new Response(null, { status: 101, webSocket: outside })
  }

  override async webSocketMessage(socket: WebSocket, message: string | ArrayBuffer) {
    if (typeof message !== "string") return
    const tags = this.ctx.getTags(socket)
    if (tags.includes("host")) {
      const routed = fromHost(message)
      if (!routed) return
      for (const peer of this.ctx.getWebSockets(`c:${routed.client}`)) peer.send(routed.data)
      return
    }
    const frame = fromClient(message, clientOf(tags))
    if (!frame) return
    if (!this.toHost(frame)) socket.send(JSON.stringify({ t: "offline" }))
  }

  override async webSocketClose(socket: WebSocket, code: number) {
    this.left(socket)
    try {
      socket.close(code === 1005 || code === 1006 ? 1000 : code)
    } catch {
      // Already closed.
    }
  }

  override async webSocketError(socket: WebSocket) {
    this.left(socket)
  }

  async kick(client: string) {
    for (const socket of this.ctx.getWebSockets(`c:${client}`)) socket.close(4001, "This device was removed.")
  }

  private left(socket: WebSocket) {
    const tags = this.ctx.getTags(socket)
    if (tags.includes("host")) {
      if (!this.ctx.getWebSockets("host").some((other) => other !== socket)) this.toClients({ t: "offline" })
      return
    }
    this.toHost({ t: "gone", c: clientOf(tags) })
    this.toHost({ t: "clients", n: this.ctx.getWebSockets("client").filter((other) => other !== socket).length })
  }

  private toHost(frame: unknown) {
    const host = newest(this.ctx.getWebSockets("host"), (socket) => this.ctx.getTags(socket))
    if (!host) return false
    host.send(typeof frame === "string" ? frame : JSON.stringify(frame))
    return true
  }

  private toClients(frame: unknown) {
    for (const socket of this.ctx.getWebSockets("client")) socket.send(JSON.stringify(frame))
  }
}

function clientOf(tags: string[]) {
  return tags.find((tag) => tag.startsWith("c:"))?.slice(2) ?? ""
}
