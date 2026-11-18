// Remote control tickets: short-lived HMAC-SHA256 tokens that the relay (services/remote-relay)
// checks without calling the gateway. REMOTE_TICKET_KEY is shared by both; keep this file in
// sync with services/remote-relay/src/ticket.ts. Format: base64url(JSON) "." base64url(MAC).

export type Ticket = {
  // "host" is a desktop engine, "client" a paired phone or browser, "kick" tells the relay to drop a client,
  // "workspace" opens a cloud workspace on the cryzo-cloud Worker (`host` is the workspace).
  role: "host" | "client" | "kick" | "workspace"
  host: string
  client?: string
  exp: number
}

export async function sign(ticket: Ticket, secret: string) {
  const body = base64url(new TextEncoder().encode(JSON.stringify(ticket)))
  return `${body}.${base64url(await mac(body, secret))}`
}

export async function verify(token: string, secret: string, now = Date.now()): Promise<Ticket | null> {
  const [body, signature, extra] = token.split(".")
  if (!body || !signature || extra !== undefined || token.length > 2048) return null
  const expected = await mac(body, secret)
  const given = unbase64url(signature)
  // Constant-time comparison over the full MAC length.
  if (!given || given.length !== expected.length) return null
  let difference = 0
  for (let index = 0; index < expected.length; index++) difference |= expected[index]! ^ given[index]!
  if (difference !== 0) return null
  const bytes = unbase64url(body)
  if (!bytes) return null
  try {
    const ticket = JSON.parse(new TextDecoder().decode(bytes)) as Ticket
    if (!["host", "client", "kick", "workspace"].includes(ticket.role) || typeof ticket.host !== "string") return null
    if (typeof ticket.exp !== "number" || ticket.exp <= now) return null
    return ticket
  } catch {
    return null
  }
}

async function mac(body: string, secret: string) {
  if (secret.length < 32) throw new Error("REMOTE_TICKET_KEY must be at least 32 characters.")
  const key = await crypto.subtle.importKey("raw", new TextEncoder().encode(secret), { name: "HMAC", hash: "SHA-256" }, false, ["sign"])
  return new Uint8Array(await crypto.subtle.sign("HMAC", key, new TextEncoder().encode(body)))
}

function base64url(bytes: Uint8Array) {
  return btoa(Array.from(bytes, (byte) => String.fromCharCode(byte)).join("")).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "")
}

function unbase64url(text: string) {
  if (!/^[A-Za-z0-9_-]*$/.test(text)) return null
  const padded = text.replace(/-/g, "+").replace(/_/g, "/") + "=".repeat((4 - (text.length % 4)) % 4)
  try {
    return Uint8Array.from(atob(padded), (char) => char.charCodeAt(0))
  } catch {
    return null
  }
}
