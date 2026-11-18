import { describe, expect, test } from "bun:test"
import { fromClient, fromHost } from "../src/route"
import { sign, verify } from "../src/ticket"
import { sign as gatewaySign } from "../../model-gateway/convex/lib/ticket"

const key = "k".repeat(48)

describe("tickets", () => {
  test("the relay accepts tickets the gateway signs, and nothing else", async () => {
    const exp = Date.now() + 60_000
    const ticket = await gatewaySign({ role: "client", host: "h1", client: "c1", exp }, key)
    expect(await verify(ticket, key)).toEqual({ role: "client", host: "h1", client: "c1", exp })
    expect(await verify(ticket, "z".repeat(48))).toBeNull()
    expect(await verify(await sign({ role: "host", host: "h1", exp: Date.now() - 1 }, key), key)).toBeNull()
  })
})

describe("routing", () => {
  test("client frames are stamped with the real sender", () => {
    expect(JSON.parse(fromClient(JSON.stringify({ t: "req", id: 1, c: "someone-else" }), "c1")!)).toEqual({ t: "req", id: 1, c: "c1" })
  })

  test("host frames go only to the addressed client", () => {
    const raw = JSON.stringify({ t: "data", id: 1, c: "c1", data: "aGk=" })
    expect(fromHost(raw)).toEqual({ client: "c1", data: raw })
    expect(fromHost(JSON.stringify({ t: "data", id: 1 }))).toBeNull()
  })

  test("malformed frames are dropped", () => {
    expect(fromClient("not json", "c1")).toBeNull()
    expect(fromClient("[1,2]", "c1")).toBeNull()
    expect(fromClient(JSON.stringify({ t: "req" }), "")).toBeNull()
    expect(fromHost("x".repeat(9 * 1024 * 1024))).toBeNull()
  })
})
