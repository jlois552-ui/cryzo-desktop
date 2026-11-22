// The relay transport: HTTP requests and terminal sockets for the user's desktop engine, carried as
// JSON frames over one WebSocket to the Cryzo relay. The frame format matches native/engine/src/remote.rs.

type Frame = {
  t: string
  id?: number
  status?: number
  headers?: [string, string][]
  data?: string
  message?: string
  code?: number
  cursor?: number
}

/** How long a request may wait for the computer to start answering. */
const ANSWER_MS = 45_000

type Pending = {
  head(status: number, headers: [string, string][]): void
  data(bytes: Uint8Array): void
  end(): void
  fail(message: string): void
}

export class Relay {
  private socket: { send(data: string): void } | undefined
  private next = 1
  private pending = new Map<number, Pending>()
  private tunnels = new Map<number, TunnelSocket>()
  online = false
  /** Called for every status the relay reports, including a repeated one. */
  onstatus: ((online: boolean) => void) | undefined

  /** Uses a newly opened relay socket; the caller passes its messages to `deliver` and its close to `detach`. */
  attach(socket: { send(data: string): void }) {
    this.socket = socket
  }

  deliver(data: unknown) {
    if (typeof data !== "string") return
    try {
      this.receive(JSON.parse(data) as Frame)
    } catch {
      // A malformed frame is dropped; the engine never sends one.
    }
  }

  /** The connection to the relay ended: every open request and terminal fails, so the app reconnects. */
  detach() {
    this.socket = undefined
    this.online = false
    for (const pending of this.pending.values()) pending.fail("The connection to your computer was lost.")
    this.pending.clear()
    for (const tunnel of this.tunnels.values()) tunnel.finish(1006)
    this.tunnels.clear()
  }

  get connected() {
    return this.socket !== undefined
  }

  async fetch(request: Request): Promise<Response> {
    if (!this.socket || !this.online) throw new TypeError("Cryzo on your computer is offline.")
    if (request.signal.aborted) throw request.signal.reason
    const id = this.next++
    const body =
      request.method === "GET" || request.method === "HEAD" ? "" : base64(new Uint8Array(await request.arrayBuffer()))
    const url = new URL(request.url)
    let controller: ReadableStreamDefaultController<Uint8Array> | undefined
    const stream = new ReadableStream<Uint8Array>({
      start: (value) => {
        controller = value
      },
      cancel: () => {
        this.pending.delete(id)
        this.send({ t: "abort", id })
      },
    })
    return new Promise<Response>((resolve, reject) => {
      let started = false
      const abort = () => {
        this.send({ t: "abort", id })
        this.pending.get(id)?.fail("aborted")
      }
      request.signal.addEventListener("abort", abort, { once: true })
      // A computer that stalls or drops the link mid-request never answers. Without a limit the
      // request (a sent message, say) waited forever with nothing on screen saying it was lost;
      // failing it lets the app report it and keep the draft.
      const silence = setTimeout(() => {
        if (started) return
        this.send({ t: "abort", id })
        this.pending.get(id)?.fail("Your computer didn't answer in time, so this didn't go through. Try again.")
      }, ANSWER_MS)
      const done = () => {
        clearTimeout(silence)
        this.pending.delete(id)
        request.signal.removeEventListener("abort", abort)
      }
      this.pending.set(id, {
        head: (status, headers) => {
          started = true
          clearTimeout(silence)
          resolve(new Response([101, 204, 205, 304].includes(status) ? null : stream, { status, headers }))
        },
        data: (bytes) => controller?.enqueue(bytes),
        end: () => {
          done()
          controller?.close()
        },
        fail: (message) => {
          done()
          const error = new TypeError(message)
          if (!started) reject(error)
          try {
            controller?.error(error)
          } catch {
            // The body was already closed or cancelled.
          }
        },
      })
      this.send({
        t: "req",
        id,
        method: request.method,
        path: url.pathname + url.search,
        headers: [...request.headers],
        body,
      })
    })
  }

  /** A terminal socket on the desktop engine, shaped like the browser's WebSocket. */
  open(path: string) {
    const id = this.next++
    const tunnel = new TunnelSocket(path, {
      send: (data) => this.send({ t: "ws-send", id, data }),
      close: () => this.send({ t: "ws-close", id }),
    })
    this.tunnels.set(id, tunnel)
    if (!this.socket || !this.online) queueMicrotask(() => tunnel.finish(1006))
    else this.send({ t: "ws", id, path })
    return tunnel
  }

  private send(frame: Record<string, unknown>) {
    this.socket?.send(JSON.stringify(frame))
  }

  private receive(frame: Frame) {
    if (frame.t === "online" || frame.t === "offline") {
      this.online = frame.t === "online"
      this.onstatus?.(this.online)
      if (frame.t === "offline") {
        for (const pending of this.pending.values()) pending.fail("Cryzo on your computer went offline.")
        this.pending.clear()
        for (const tunnel of this.tunnels.values()) tunnel.finish(1006)
        this.tunnels.clear()
      }
      return
    }
    const id = frame.id ?? 0
    if (frame.t.startsWith("ws-")) {
      const tunnel = this.tunnels.get(id)
      if (!tunnel) return
      if (frame.t === "ws-open") tunnel.opened()
      if (frame.t === "ws-data") tunnel.message(frame.data ?? "")
      if (frame.t === "ws-cursor") tunnel.cursor(frame.cursor ?? 0)
      if (frame.t === "ws-close") {
        this.tunnels.delete(id)
        tunnel.finish(frame.code ?? 1000)
      }
      return
    }
    const pending = this.pending.get(id)
    if (!pending) return
    if (frame.t === "head") pending.head(frame.status ?? 502, frame.headers ?? [])
    if (frame.t === "data") pending.data(unbase64(frame.data ?? ""))
    if (frame.t === "end") pending.end()
    if (frame.t === "error") pending.fail(frame.message ?? "The request failed on your computer.")
  }
}

/** Matches the desktop's native terminal sockets, so the upstream terminal component works unchanged. */
export class TunnelSocket extends EventTarget {
  readonly url: string
  readonly protocol = ""
  readonly extensions = ""
  readonly CONNECTING = 0
  readonly OPEN = 1
  readonly CLOSING = 2
  readonly CLOSED = 3
  binaryType: BinaryType = "blob"
  readyState = 0
  bufferedAmount = 0
  onopen: ((event: Event) => void) | null = null
  onmessage: ((event: MessageEvent) => void) | null = null
  onerror: ((event: Event) => void) | null = null
  onclose: ((event: CloseEvent) => void) | null = null

  constructor(
    url: string,
    private link: { send(data: string): void; close(): void },
  ) {
    super()
    this.url = url
  }

  send(data: string | ArrayBufferLike | Blob | ArrayBufferView) {
    if (this.readyState !== 1) throw new DOMException("Terminal socket is not open", "InvalidStateError")
    if (data instanceof Blob) {
      void data.arrayBuffer().then((buffer) => this.link.send(base64(new Uint8Array(buffer))))
      return
    }
    const bytes =
      typeof data === "string"
        ? new TextEncoder().encode(data)
        : ArrayBuffer.isView(data)
          ? new Uint8Array(data.buffer, data.byteOffset, data.byteLength)
          : new Uint8Array(data)
    this.link.send(base64(bytes))
  }

  close(code = 1000, reason = "") {
    if (this.readyState >= 2) return
    this.readyState = 2
    this.link.close()
    this.finish(code, reason)
  }

  opened() {
    if (this.readyState !== 0) return
    this.readyState = 1
    const event = new Event("open")
    this.dispatchEvent(event)
    this.onopen?.(event)
  }

  message(data: string | ArrayBuffer | Blob) {
    if (this.readyState !== 1) return
    const event = new MessageEvent("message", { data })
    this.dispatchEvent(event)
    this.onmessage?.(event)
  }

  cursor(cursor: number) {
    const bytes = new TextEncoder().encode(`\0${JSON.stringify({ cursor })}`)
    this.message(this.binaryType === "arraybuffer" ? bytes.buffer : new Blob([bytes]))
  }

  finish(code: number, reason = "") {
    if (this.readyState === 3) return
    this.readyState = 3
    const event = new CloseEvent("close", { code, reason, wasClean: code === 1000 })
    this.dispatchEvent(event)
    this.onclose?.(event)
  }
}

function base64(bytes: Uint8Array) {
  let text = ""
  for (let index = 0; index < bytes.length; index += 0x8000)
    text += String.fromCharCode(...bytes.subarray(index, index + 0x8000))
  return btoa(text)
}

function unbase64(text: string) {
  return Uint8Array.from(atob(text), (char) => char.charCodeAt(0))
}
