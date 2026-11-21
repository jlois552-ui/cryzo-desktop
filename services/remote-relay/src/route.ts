// Frame routing between one desktop engine (the host) and its paired clients. Frames are JSON
// text. The relay reads only the `c` field: the client a frame belongs to.

const maxFrame = 8 * 1024 * 1024

/** A client's frame for the host, stamped with the sender so a client cannot speak for another. */
export function fromClient(raw: string, client: string) {
  const frame = parse(raw)
  if (!frame || !client) return null
  return JSON.stringify({ ...frame, c: client })
}

/** The client a host frame is addressed to, and the frame to deliver unchanged. */
export function fromHost(raw: string) {
  const frame = parse(raw)
  if (!frame || typeof frame.c !== "string" || !frame.c) return null
  return { client: frame.c, data: raw }
}

function parse(raw: string) {
  if (raw.length > maxFrame) return null
  try {
    const value = JSON.parse(raw) as unknown
    return value && typeof value === "object" && !Array.isArray(value) ? (value as Record<string, unknown>) : null
  } catch {
    return null
  }
}

/**
 * The most recently connected engine socket. A replaced socket can still be listed while its close
 * completes, and requests sent to it would never be answered.
 */
export function newest<Socket>(sockets: Socket[], tags: (socket: Socket) => string[]) {
  const connected = (socket: Socket) => Number(tags(socket).find((tag) => tag.startsWith("h:"))?.slice(2) ?? 0)
  return sockets.reduce<Socket | undefined>((best, socket) => (!best || connected(socket) > connected(best) ? socket : best), undefined)
}
