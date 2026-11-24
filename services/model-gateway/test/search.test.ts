import { expect, test } from "bun:test"
import { handleSearch } from "../convex/search"

const request = () => new Request("https://example.test/search/web", { method: "POST", body: JSON.stringify({ query: "  Rust release  " }) })
const unused = () => { throw new Error("unexpected upstream request") }

test("search separates sign-in, unconfigured service and quota exhaustion", async () => {
  expect((await handleSearch(request(), null, "test", unused, fetch)).status).toBe(401)
  expect((await handleSearch(request(), "user", undefined, unused, fetch)).status).toBe(503)
  expect((await handleSearch(request(), "user", "test", async () => false, fetch)).status).toBe(429)
})

test("search returns bounded source links from Brave", async () => {
  let calls = 0
  const send = Object.assign(async (url: RequestInfo | URL, init?: RequestInit) => {
    calls++
    expect(new URL(String(url)).searchParams.get("q")).toBe("Rust release")
    expect(new Headers(init?.headers).get("X-Subscription-Token")).toBe("fixture")
    return Response.json({ web: { results: [
      { title: "Release", url: "https://example.test/source", description: "Source" },
      { url: "javascript:void(0)" },
    ] } })
  }, { preconnect() {} })
  const res = await handleSearch(request(), "user", "fixture", async (id) => id === "user", send)
  expect(await res.json()).toEqual({ data: { results: [{ title: "Release", url: "https://example.test/source", snippet: "Source" }] } })
  expect(calls).toBe(1)
})
