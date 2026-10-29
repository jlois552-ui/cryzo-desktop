import { beforeEach, expect, test } from "bun:test"
import { convexTest } from "convex-test"
import schema from "../convex/schema"
import { internal } from "../convex/_generated/api"
import { availableModels, inferenceBody, modelIDs } from "../convex/lib/models"

const modules = { "../convex/quota.ts": () => import("../convex/quota"), "../convex/_generated/server.js": () => import("../convex/_generated/server") }
beforeEach(() => { delete process.env.FREE_MAX_CONCURRENT; delete process.env.FREE_REQUESTS_PER_MINUTE })
function setup() {
  const t = convexTest(schema, modules)
  const actor = (user: string) => ({
    reserve: (step: string) => t.mutation(internal.quota.reserve, { user, step }),
    finish: (step: string, retryAfter?: number) => t.mutation(internal.quota.finish, { user, step, ...(retryAfter === undefined ? {} : { retryAfter }) }),
  })
  return { t, actor, user: actor("alice") }
}
test("catalog exposes only curated live NVIDIA IDs", () => {
  expect(availableModels({ data: [{ id: modelIDs[0] }, { id: "z-ai/glm-5.3" }, { id: "big-pickle" }, { id: "nvidia/embed" }] }).map((m) => m.id)).toEqual([modelIDs[0]])
  expect(() => availableModels("Bad preview URL")).toThrow()
})
test("gateway pins NVIDIA models and drops arbitrary routing and billing options", () => {
  const result = inferenceBody({ model: modelIDs[0], messages: [{ role: "user", content: "hello" }], cryzo_step_id: "msg_fixture", provider: { order: ["paid"] }, max_tokens: 9999999, stream: false })
  expect(result.body.stream).toBe(true)
  expect(result.body.max_tokens).toBe(16384)
  expect(result.body.provider).toBeUndefined()
  expect(() => inferenceBody({ model: "paid-model" })).toThrow("unsupported free model")
})
test("catalog keeps exactly the six curated free models", () => {
  expect(modelIDs).toHaveLength(6)
  expect(availableModels({ data: modelIDs.map((id) => ({ id })) }).map((m) => m.id)).toEqual([...modelIDs])
  for (const id of modelIDs) expect(inferenceBody({ model: id, messages: [{ role: "user", content: "hi" }], cryzo_step_id: "msg_x" }).body.model).toBe(id)
})
test("duplicate steps cannot invoke the provider again or consume another call", async () => {
  const { user } = setup()
  expect(await user.reserve("msg_one")).toEqual({ remaining: 99 })
  await user.finish("msg_one")
  await expect(user.reserve("msg_one")).rejects.toThrow("duplicate step")
  expect(await user.reserve("msg_two")).toEqual({ remaining: 98 })
})
test("active user and global pool limits are enforced transactionally", async () => {
  const { t, actor, user } = setup()
  await user.reserve("msg_one")
  await expect(user.reserve("msg_two")).rejects.toThrow("one generation")
  const bob = actor("bob")
  await bob.reserve("msg_three")
  const claire = actor("claire")
  await expect(claire.reserve("msg_four")).rejects.toThrow("free service busy")
  await expect(bob.finish("msg_one")).rejects.toThrow("unknown step")
})
test("100-call daily cap and global NVIDIA retry-after are enforced", async () => {
  const { t, actor, user } = setup()
  await t.run(async (ctx) => { await ctx.db.insert("usage", { user: "alice", day: new Date().toISOString().slice(0, 10), calls: 99 }) })
  expect(await user.reserve("msg_last")).toEqual({ remaining: 0 })
  await user.finish("msg_last", 60)
  await expect(user.reserve("msg_excess")).rejects.toThrow("daily limit")
  const bob = actor("bob")
  await expect(bob.reserve("msg_wait")).rejects.toThrow("free service busy")
})
