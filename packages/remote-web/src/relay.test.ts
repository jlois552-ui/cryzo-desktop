import { describe, expect, test } from "bun:test"
import { Relay } from "./relay"

// Plays the engine's side of the relay socket: records what the page sends, delivers frames back.
function engine() {
  const sent: Record<string, unknown>[] = []
  const relay = new Relay()
  relay.attach({ send: (data) => sent.push(JSON.parse(data) as Record<string, unknown>) })
  const deliver = (frame: unknown) => relay.deliver(JSON.stringify(frame))
  deliver({ t: "online" })
  return { relay, sent, deliver }
}

const b64 = (text: string) => btoa(text)

describe("relay transport", () => {
  test("a request becomes one frame and its streamed reply becomes a Response", async () => {
    const { relay, sent, deliver } = engine()
    const pending = relay.fetch(
      new Request("https://cryzo.example/api/session?x=1", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: '{"a":1}',
      }),
    )
    await Bun.sleep(0)
    expect(sent[0]).toMatchObject({ t: "req", id: 1, method: "POST", path: "/api/session?x=1", body: b64('{"a":1}') })
    deliver({ t: "head", id: 1, status: 201, headers: [["content-type", "text/plain"]] })
    deliver({ t: "data", id: 1, data: b64("hel") })
    deliver({ t: "data", id: 1, data: b64("lo") })
    deliver({ t: "end", id: 1 })
    const response = await pending
    expect(response.status).toBe(201)
    expect(response.headers.get("content-type")).toBe("text/plain")
    expect(await response.text()).toBe("hello")
  })

  test("an engine error rejects the request", async () => {
    const { relay, deliver } = engine()
    const pending = relay.fetch(new Request("https://cryzo.example/api/x"))
    await Bun.sleep(0)
    deliver({ t: "error", id: 1, message: "remote clients can only use the session API" })
    await expect(pending).rejects.toThrow("session API")
  })

  test("aborting a request tells the engine to stop it", async () => {
    const { relay, sent } = engine()
    const controller = new AbortController()
    const pending = relay.fetch(new Request("https://cryzo.example/api/event", { signal: controller.signal }))
    await Bun.sleep(0)
    controller.abort()
    await expect(pending).rejects.toThrow()
    expect(sent.at(-1)).toEqual({ t: "abort", id: 1 })
  })

  test("losing the computer fails open requests, so the app reconnects", async () => {
    const { relay, deliver } = engine()
    const pending = relay.fetch(new Request("https://cryzo.example/api/event"))
    await Bun.sleep(0)
    deliver({ t: "offline" })
    await expect(pending).rejects.toThrow("offline")
    expect(relay.online).toBe(false)
    await expect(relay.fetch(new Request("https://cryzo.example/api/x"))).rejects.toThrow("offline")
  })

  test("terminal sockets open, receive output and cursors, and send input", async () => {
    const { relay, sent, deliver } = engine()
    const socket = relay.open("/api/pty/p1/connect?ticket=t")
    socket.binaryType = "arraybuffer"
    const messages: unknown[] = []
    socket.addEventListener("message", (event) => messages.push((event as MessageEvent).data))
    expect(sent[0]).toEqual({ t: "ws", id: 1, path: "/api/pty/p1/connect?ticket=t" })
    deliver({ t: "ws-open", id: 1 })
    expect(socket.readyState).toBe(1)
    deliver({ t: "ws-data", id: 1, data: "$ " })
    deliver({ t: "ws-cursor", id: 1, cursor: 2 })
    expect(messages[0]).toBe("$ ")
    expect(new TextDecoder().decode(messages[1] as ArrayBuffer)).toBe('\0{"cursor":2}')
    socket.send("ls\r")
    expect(sent.at(-1)).toEqual({ t: "ws-send", id: 1, data: b64("ls\r") })
    let closed = 0
    socket.addEventListener("close", (event) => (closed = (event as CloseEvent).code))
    deliver({ t: "ws-close", id: 1, code: 1000 })
    expect(closed).toBe(1000)
  })
})
