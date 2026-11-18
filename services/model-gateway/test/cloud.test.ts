import { afterEach, describe, expect, setSystemTime, test } from "bun:test"
import { convexTest } from "convex-test"
import schema from "../convex/schema"
import { internal } from "../convex/_generated/api"
import { allowance, caps, sandboxName } from "../convex/lib/cloud"

const modules = {
  "../convex/cloud/store.ts": () => import("../convex/cloud/store"),
  "../convex/_generated/server.js": () => import("../convex/_generated/server"),
}

process.env.CLOUD_FREE_DAILY_MINUTES = "30"
process.env.CLOUD_MONTHLY_MINUTES = "45"

afterEach(() => setSystemTime())

describe("cloud caps", () => {
  test("defaults keep a small free allowance and a hard monthly budget", () => {
    expect(caps({})).toEqual({ dailyMinutes: 30, monthlyMinutes: 600 })
    expect(caps({ CLOUD_FREE_DAILY_MINUTES: "-1", CLOUD_MONTHLY_MINUTES: "x" })).toEqual({ dailyMinutes: 30, monthlyMinutes: 600 })
  })

  test("the smaller of the user's day and the service's month wins", () => {
    const limits = { dailyMinutes: 30, monthlyMinutes: 600 }
    expect(allowance({ usedToday: 10, usedMonth: 0, caps: limits })).toEqual({ ok: true, minutes: 20 })
    expect(allowance({ usedToday: 0, usedMonth: 590, caps: limits })).toEqual({ ok: true, minutes: 10 })
    expect(allowance({ usedToday: 30, usedMonth: 0, caps: limits }).ok).toBe(false)
    expect(allowance({ usedToday: 0, usedMonth: 600, caps: limits })).toMatchObject({ ok: false, reason: expect.stringContaining("everyone") })
  })

  test("each user and project gets its own stable sandbox name", async () => {
    expect(await sandboxName("u1", "p")).toBe(await sandboxName("u1", "p"))
    expect(await sandboxName("u1", "p")).not.toBe(await sandboxName("u2", "p"))
    expect(await sandboxName("u1", "p")).toMatch(/^cryzo-[0-9a-f]{24}$/)
  })
})

describe("cloud usage", () => {
  test("running time is counted per day and month, and a spent day blocks new starts", async () => {
    setSystemTime(new Date("2026-11-16T10:00:00Z"))
    const t = convexTest(schema, modules)
    const workspace = { user: "u1", project: "app" }
    expect((await t.mutation(internal.cloud.store.reserve, workspace)).minutes).toBe(30)
    await t.mutation(internal.cloud.store.started, { ...workspace, url: "https://agent", previews: {} })

    setSystemTime(new Date("2026-11-16T10:20:00Z"))
    const beat = await t.mutation(internal.cloud.store.heartbeat, { ...workspace, active: true })
    expect(beat).toMatchObject({ running: true, allowed: { ok: true, minutes: 10 } })

    setSystemTime(new Date("2026-11-16T10:31:00Z"))
    await t.mutation(internal.cloud.store.stopped, workspace)
    expect(await t.query(internal.cloud.store.get, workspace)).toMatchObject({ status: "stopped", usedToday: 31, url: null })
    await expect(t.mutation(internal.cloud.store.reserve, workspace)).rejects.toThrow(/today's free cloud time/)

    // A new day resets the user's allowance, but the service's month keeps counting.
    setSystemTime(new Date("2026-11-17T09:00:00Z"))
    expect((await t.mutation(internal.cloud.store.reserve, workspace)).minutes).toBe(14)
  })

  test("a stopped workspace adds no time", async () => {
    setSystemTime(new Date("2026-11-16T10:00:00Z"))
    const t = convexTest(schema, modules)
    const workspace = { user: "u1", project: "app" }
    await t.mutation(internal.cloud.store.reserve, workspace)
    await t.mutation(internal.cloud.store.started, { ...workspace, url: "https://agent", previews: {} })
    await t.mutation(internal.cloud.store.stopped, workspace)
    setSystemTime(new Date("2026-11-16T12:00:00Z"))
    expect(await t.mutation(internal.cloud.store.heartbeat, { ...workspace, active: true })).toEqual({ running: false })
    expect((await t.query(internal.cloud.store.get, workspace)).usedToday).toBe(0)
  })
})
