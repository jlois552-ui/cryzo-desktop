import { describe, expect, test } from "bun:test"
import { convexTest } from "convex-test"
import schema from "../convex/schema"
import { internal } from "../convex/_generated/api"
import { manifestProblem, readAssetRouting, readModules, secretProblem, slugProblem } from "../convex/lib/apps"

const modules = {
  "../convex/apps/store.ts": () => import("../convex/apps/store"),
  "../convex/_generated/server.js": () => import("../convex/_generated/server"),
}

const hash = "0123456789abcdef0123456789abcdef"

describe("app addresses", () => {
  test("only hostname-safe, unreserved addresses are accepted", () => {
    expect(slugProblem("my-todo-app")).toBeUndefined()
    expect(slugProblem("ab")).toBeDefined()
    expect(slugProblem("My-App")).toBeDefined()
    expect(slugProblem("-app")).toBeDefined()
    expect(slugProblem("a--b")).toBeDefined()
    expect(slugProblem("www")).toBeDefined()
  })

  test("manifests must list safe paths with Cloudflare-style hashes", () => {
    expect(manifestProblem({ "/index.html": { hash, size: 10 } })).toBeUndefined()
    expect(manifestProblem({})).toBeDefined()
    expect(manifestProblem({ "index.html": { hash, size: 10 } })).toBeDefined()
    expect(manifestProblem({ "/../x": { hash, size: 10 } })).toBeDefined()
    expect(manifestProblem({ "/a.js": { hash: "xyz", size: 10 } })).toBeDefined()
    expect(manifestProblem({ "/a.mp4": { hash, size: 26 * 1024 * 1024 } })).toBeDefined()
  })
})

describe("published apps", () => {
  test("an address belongs to its first owner, who can republish it", async () => {
    const t = convexTest(schema, modules)
    await t.mutation(internal.apps.store.claim, { user: "ana", slug: "notes", script: "app-notes" })
    await t.mutation(internal.apps.store.update, { user: "ana", slug: "notes", database: "db1" })
    const again = await t.mutation(internal.apps.store.claim, { user: "ana", slug: "notes", script: "app-notes" })
    expect(again.database).toBe("db1")
    await expect(t.mutation(internal.apps.store.claim, { user: "ben", slug: "notes", script: "app-notes" })).rejects.toThrow("taken")
    expect(await t.query(internal.apps.store.get, { user: "ben", slug: "notes" })).toBeNull()
  })

  test("the free plan keeps one app live until it is unpublished", async () => {
    const t = convexTest(schema, modules)
    await t.mutation(internal.apps.store.claim, { user: "ana", slug: "notes", script: "app-notes" })
    await expect(t.mutation(internal.apps.store.claim, { user: "ana", slug: "shop", script: "app-shop" })).rejects.toThrow("Unpublish notes")
    await t.mutation(internal.apps.store.remove, { user: "ana", slug: "notes" })
    await t.mutation(internal.apps.store.claim, { user: "ana", slug: "shop", script: "app-shop" })
    expect(await t.query(internal.apps.store.list, { user: "ana" })).toEqual([{ slug: "shop", url: null, publishedAt: null }])
  })
})

describe("app server code", () => {
  test("modules keep their order and must have safe, unique names", () => {
    const code = [
      { name: "index.js", content: "import('./assets/a.js')" },
      { name: "assets/a.js", content: "export {}" },
    ]
    expect(readModules(code)).toEqual(code)
    expect(readModules(undefined)).toEqual([])
    expect(() => readModules([{ name: "../index.js", content: "" }])).toThrow()
    expect(() => readModules([{ name: "/index.js", content: "" }])).toThrow()
    expect(() => readModules([{ name: "index.ts", content: "" }])).toThrow()
    expect(() => readModules([code[0], code[0]])).toThrow()
  })

  test("routing keeps known settings and defaults to a single-page app", () => {
    expect(readAssetRouting({ not_found_handling: "single-page-application", run_worker_first: ["/api/*", "!/api/docs"] })).toEqual({
      not_found_handling: "single-page-application",
      run_worker_first: ["/api/*", "!/api/docs"],
    })
    expect(readAssetRouting({ not_found_handling: "bogus", run_worker_first: ["api"], directory: "../client" })).toEqual({
      not_found_handling: "single-page-application",
    })
    expect(readAssetRouting(undefined)).toEqual({ not_found_handling: "single-page-application" })
  })
})

describe("app secrets", () => {
  test("names are environment-style and Cryzo's own can't be replaced", () => {
    expect(secretProblem("OPENAI_API_KEY", "value")).toBeUndefined()
    expect(secretProblem("GOOGLE_CLIENT_ID")).toBeUndefined()
    expect(secretProblem("openai_key", "value")).toBeDefined()
    expect(secretProblem("1KEY", "value")).toBeDefined()
    expect(secretProblem("BETTER_AUTH_SECRET", "value")).toBeDefined()
    expect(secretProblem("APP_URL")).toBeDefined()
    expect(secretProblem("TOKEN", "")).toBeDefined()
    expect(secretProblem("TOKEN", "x".repeat(5 * 1024 + 1))).toBeDefined()
  })
})
