import { describe, expect, setSystemTime, test, afterEach } from "bun:test"
import { convexTest } from "convex-test"
import schema from "../convex/schema"
import { internal } from "../convex/_generated/api"
import { sign, verify } from "../convex/lib/ticket"
import { digest, normalizeCode } from "../convex/remote/http"

const modules = {
  "../convex/remote/store.ts": () => import("../convex/remote/store"),
  "../convex/_generated/server.js": () => import("../convex/_generated/server"),
}

const key = "k".repeat(48)

afterEach(() => setSystemTime())

describe("relay tickets", () => {
  test("a signed ticket verifies until it expires", async () => {
    setSystemTime(new Date("2026-11-16T12:00:00Z"))
    const exp = Date.now() + 60_000
    const ticket = await sign({ role: "client", host: "h1", client: "c1", exp }, key)
    expect(await verify(ticket, key)).toEqual({ role: "client", host: "h1", client: "c1", exp })
    expect(await verify(ticket, key, exp)).toBeNull()
  })

  test("a forged, tampered or differently keyed ticket is rejected", async () => {
    const ticket = await sign({ role: "host", host: "h1", exp: Date.now() + 60_000 }, key)
    const [body, mac] = ticket.split(".")
    const forged = btoa(JSON.stringify({ role: "host", host: "h2", exp: Date.now() + 60_000 })).replace(/=+$/, "")
    expect(await verify(`${forged}.${mac}`, key)).toBeNull()
    expect(await verify(`${body}.${mac!.slice(0, -2)}AA`, key)).toBeNull()
    expect(await verify(ticket, "x".repeat(48))).toBeNull()
    expect(await verify(`${ticket}.extra`, key)).toBeNull()
  })
})

describe("pairing", () => {
  test("codes ignore case and separators", () => {
    expect(normalizeCode(" k7qf-2m9x ")).toBe("K7QF2M9X")
  })

  test("a code pairs one device once, and revoking removes its access", async () => {
    setSystemTime(new Date("2026-11-16T12:00:00Z"))
    const t = convexTest(schema, modules)
    const host = await t.mutation(internal.remote.store.createHost, { user: "u1", name: "Desk", secretHash: await digest("host-secret") })
    const codeHash = await digest("K7QF2M9X")
    await t.mutation(internal.remote.store.pair, { user: "u1", host, codeHash })

    const claimed = await t.mutation(internal.remote.store.claim, { codeHash, name: "Phone", secretHash: await digest("client-secret") })
    expect(claimed.host).toBe(host)
    await expect(t.mutation(internal.remote.store.claim, { codeHash, name: "Other", secretHash: await digest("x") })).rejects.toThrow(/expired or was already used/)

    expect(await t.mutation(internal.remote.store.authenticate, { role: "host", id: host, secretHash: await digest("host-secret") })).toEqual({ host })
    expect(await t.mutation(internal.remote.store.authenticate, { role: "host", id: host, secretHash: await digest("wrong") })).toBeNull()
    expect(
      await t.mutation(internal.remote.store.authenticate, { role: "client", id: claimed.client, secretHash: await digest("client-secret") }),
    ).toEqual({ host, client: claimed.client })

    expect(await t.query(internal.remote.store.clients, { user: "u1", host })).toEqual([
      { id: claimed.client, name: "Phone", createdAt: Date.now(), lastSeenAt: Date.now() },
    ])
    await expect(t.query(internal.remote.store.clients, { user: "u2", host })).rejects.toThrow(/not set up/)
    await expect(t.mutation(internal.remote.store.revoke, { user: "u2", client: claimed.client })).rejects.toThrow(/not paired/)

    await t.mutation(internal.remote.store.revoke, { user: "u1", client: claimed.client })
    expect(
      await t.mutation(internal.remote.store.authenticate, { role: "client", id: claimed.client, secretHash: await digest("client-secret") }),
    ).toBeNull()
  })

  test("an expired code cannot be redeemed, and a new code replaces the old one", async () => {
    setSystemTime(new Date("2026-11-16T12:00:00Z"))
    const t = convexTest(schema, modules)
    const host = await t.mutation(internal.remote.store.createHost, { user: "u1", name: "Desk", secretHash: "h" })
    const first = await digest("AAAA1111")
    await t.mutation(internal.remote.store.pair, { user: "u1", host, codeHash: first })
    const second = await digest("BBBB2222")
    await t.mutation(internal.remote.store.pair, { user: "u1", host, codeHash: second })
    await expect(t.mutation(internal.remote.store.claim, { codeHash: first, name: "P", secretHash: "s" })).rejects.toThrow(/expired/)

    setSystemTime(new Date("2026-11-16T12:05:01Z"))
    await expect(t.mutation(internal.remote.store.claim, { codeHash: second, name: "P", secretHash: "s" })).rejects.toThrow(/expired/)
  })

  test("removing a host removes its devices", async () => {
    const t = convexTest(schema, modules)
    const host = await t.mutation(internal.remote.store.createHost, { user: "u1", name: "Desk", secretHash: "h" })
    await t.mutation(internal.remote.store.pair, { user: "u1", host, codeHash: "c" })
    const { client } = await t.mutation(internal.remote.store.claim, { codeHash: "c", name: "P", secretHash: "s" })
    await t.mutation(internal.remote.store.removeHost, { user: "u1", host })
    expect(await t.mutation(internal.remote.store.authenticate, { role: "client", id: client, secretHash: "s" })).toBeNull()
    expect(await t.mutation(internal.remote.store.authenticate, { role: "host", id: host, secretHash: "h" })).toBeNull()
  })
})
