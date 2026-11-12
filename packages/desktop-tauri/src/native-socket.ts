import { Channel, invoke } from "@tauri-apps/api/core"

type SocketEvent =
  | { kind: "open" }
  | { kind: "data"; data: string }
  | { kind: "cursor"; cursor: number }
  | { kind: "close"; code: number }

// Adapt only the in-process engine's socket URLs. Other servers keep the browser's
// WebSocket implementation; the original terminal component stays unchanged.
export function installNativeSockets(origin: string) {
  const base = new URL(origin)
  base.protocol = base.protocol === "https:" ? "wss:" : "ws:"
  window.WebSocket = new Proxy(window.WebSocket, {
    construct(target, args) {
      const url = new URL(String(args[0]), window.location.href)
      if (url.origin !== base.origin) return new target(...(args as ConstructorParameters<typeof WebSocket>))
      return new NativeSocket(url.href)
    },
  })
}

class NativeSocket extends EventTarget {
  readonly id = crypto.randomUUID()
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
  private writes = Promise.resolve()

  constructor(url: string) {
    super()
    this.url = url
    const channel = new Channel<SocketEvent>((event) => {
      if (event.kind === "close") {
        this.finish(event.code)
        return
      }
      if (this.readyState >= 2) {
        void invoke("native_socket_close", { id: this.id })
        return
      }
      if (event.kind === "open") {
        this.readyState = 1
        const opened = new Event("open")
        this.dispatchEvent(opened)
        this.onopen?.(opened)
        return
      }
      const data = event.kind === "data" ? event.data : this.control(event.cursor)
      const message = new MessageEvent("message", { data, origin: new URL(url).origin })
      this.dispatchEvent(message)
      this.onmessage?.(message)
    })
    void invoke("native_socket_open", { id: this.id, url, channel }).catch(() => {
      if (this.readyState === 3) return
      const error = new Event("error")
      this.dispatchEvent(error)
      this.onerror?.(error)
      this.finish(1011)
    })
  }
  send(data: string | ArrayBufferLike | Blob | ArrayBufferView) {
    if (this.readyState !== 1) throw new DOMException("Terminal socket is not open", "InvalidStateError")
    const size =
      typeof data === "string"
        ? new TextEncoder().encode(data).byteLength
        : data instanceof Blob
          ? data.size
          : data.byteLength
    this.bufferedAmount += size
    this.writes = this.writes
      .then(async () => {
        const bytes =
          typeof data === "string"
            ? new TextEncoder().encode(data)
            : data instanceof Blob
              ? new Uint8Array(await data.arrayBuffer())
              : ArrayBuffer.isView(data)
                ? new Uint8Array(data.buffer, data.byteOffset, data.byteLength)
                : new Uint8Array(data)
        await invoke("native_socket_send", { id: this.id, data: [...bytes] })
      })
      .catch(() => this.finish(1011))
      .finally(() => {
        this.bufferedAmount -= size
      })
  }
  close(code = 1000, reason = "") {
    if (this.readyState >= 2) return
    if (code !== 1000 && (code < 3000 || code > 4999))
      throw new DOMException("Invalid close code", "InvalidAccessError")
    this.readyState = 2
    void this.writes.finally(() =>
      invoke("native_socket_close", { id: this.id }).finally(() => this.finish(code, reason)),
    )
  }
  private control(cursor: number) {
    const bytes = new TextEncoder().encode(`\0${JSON.stringify({ cursor })}`)
    return this.binaryType === "arraybuffer" ? bytes.buffer : new Blob([bytes])
  }
  private finish(code: number, reason = "") {
    if (this.readyState === 3) return
    this.readyState = 3
    const event = new CloseEvent("close", { code, reason, wasClean: code === 1000 })
    this.dispatchEvent(event)
    this.onclose?.(event)
  }
}
