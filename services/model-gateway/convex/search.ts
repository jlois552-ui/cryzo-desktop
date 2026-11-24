import { internal } from "./_generated/api"
import { httpAction } from "./_generated/server"
import { account } from "./gateway"

const headers = { "Content-Type": "application/json", "Cache-Control": "no-store" }
const reply = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers })

export const search = httpAction(async (ctx, request) => {
  return handleSearch(request, await account(ctx, request), process.env.BRAVE_SEARCH_API_KEY,
    (user) => ctx.runMutation(internal.quota.admitSearch, { user }), fetch)
})

export async function handleSearch(request: Request, user: string | null, secret: string | undefined,
  admit: (user: string) => Promise<boolean>, send: typeof fetch) {
  if (!user) return reply({ error: "Sign in to Cryzo to use web search." }, 401)
  const key = secret?.trim()
  if (!key) return reply({ error: "Cryzo web search hasn't been configured yet." }, 503)
  const input: { query?: unknown } | null = await request.json().catch(() => null)
  const query = typeof input?.query === "string" ? input.query.trim() : ""
  if (!query || [...query].length > 400 || query.split(/\s+/).length > 50) {
    return reply({ error: "Use a web search query between 1 and 400 characters, at most 50 words." }, 400)
  }
  if (!(await admit(user))) {
    return reply({ error: "Cryzo's daily web search limit has been reached. Try again tomorrow (UTC)." }, 429)
  }

  const url = new URL("https://api.search.brave.com/res/v1/web/search")
  url.searchParams.set("q", query)
  url.searchParams.set("count", "6")
  url.searchParams.set("text_decorations", "false")
  const response = await send(url, {
    headers: { Accept: "application/json", "X-Subscription-Token": key },
    redirect: "error",
    signal: AbortSignal.timeout(12_000),
  }).catch(() => undefined)
  if (response?.status === 429) return reply({ error: "Search is temporarily rate-limited. Try again later." }, 429)
  if (!response?.ok) return reply({ error: "Cryzo's search provider is temporarily unavailable." }, 502)
  const data: unknown = await response.json().catch(() => undefined)
  const results = searchResults(data)
  if (!results) return reply({ error: "Cryzo's search provider sent an invalid response." }, 502)
  return reply({ data: { results } })
}

function searchResults(data: unknown) {
  if (!data || typeof data !== "object") return undefined
  if (!("web" in data)) return "type" in data && data.type === "search" ? [] : undefined
  if (!data.web || typeof data.web !== "object" || !("results" in data.web) || !Array.isArray(data.web.results)) return undefined
  return data.web.results.slice(0, 6).flatMap((item: unknown) => {
    if (!item || typeof item !== "object" || !("url" in item) || typeof item.url !== "string") return []
    if (!URL.canParse(item.url) || !["https:", "http:"].includes(new URL(item.url).protocol)) return []
    return [{
      url: item.url,
      title: "title" in item && typeof item.title === "string" ? item.title.slice(0, 240) : "",
      snippet: "description" in item && typeof item.description === "string" ? item.description.slice(0, 800) : "",
    }]
  })
}
