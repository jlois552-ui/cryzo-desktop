// Reddit: permanent OAuth grant and text submissions. Reddit approves API access by hand, so the
// network stays listed under MARKET_PENDING until the app is approved.
import { json, link, ProviderError, tokens, type Init, type Provider } from "./net"

// Reddit requires a descriptive User-Agent on every call.
const agent = { "User-Agent": "web:me.cryzo.desktop:1.0 (by /u/cryzo_app)" }
const auth = (token: string): Init => ({ token, headers: agent })

export const reddit: Provider = {
  env: ["REDDIT_CLIENT_ID", "REDDIT_CLIENT_SECRET"],
  pkce: false,
  authorize: (input) =>
    link("https://www.reddit.com/api/v1/authorize", {
      client_id: input.client.id,
      response_type: "code",
      state: input.state,
      redirect_uri: input.redirect,
      duration: "permanent",
      scope: "identity submit read flair",
    }),
  exchange: async (input) =>
    tokens(
      await json("https://www.reddit.com/api/v1/access_token", {
        method: "POST",
        headers: agent,
        basic: [input.client.id, input.client.secret],
        form: { grant_type: "authorization_code", code: input.code, redirect_uri: input.redirect },
      }),
    ),
  refresh: async (current, client) =>
    tokens(
      await json("https://www.reddit.com/api/v1/access_token", {
        method: "POST",
        headers: agent,
        basic: [client.id, client.secret],
        form: { grant_type: "refresh_token", refresh_token: current.refresh ?? "" },
      }),
      current,
    ),
  profile: async (current) => {
    const me = await json("https://oauth.reddit.com/api/v1/me", auth(current.access))
    return { id: String(me.id), name: `u/${me.name}`, avatar: String(me.icon_img ?? "").replace(/&amp;/g, "&") || undefined }
  },
  publish: async (job, send) => {
    const posted = await (
      await send(
        "https://oauth.reddit.com/api/submit",
        {
          method: "POST",
          ...auth(job.tokens.access),
          form: {
            api_type: "json",
            sr: job.options.redditCommunity?.replace(/^r\//, "") ?? "",
            kind: "self",
            title: job.content.split("\n")[0].slice(0, 300),
            text: job.content,
          },
        },
        true,
      )
    ).json()
    // Reddit reports validation errors with HTTP 200; nothing was posted.
    const errors: unknown[][] = posted.json?.errors ?? []
    if (errors.length) throw new ProviderError(400, `Reddit: ${errors.map((error) => error[1] ?? error[0]).join("; ")}`, true)
    return { id: String(posted.json.data.name ?? posted.json.data.id), url: posted.json.data.url }
  },
  metrics: async () => ({ status: "unsupported", note: "Reddit does not provide account analytics.", metrics: [], series: [] }),
}
