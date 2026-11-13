// X: OAuth 2.0 with PKCE, v2 posts and chunked v2 media upload. Pay-per-use applies to
// every call, including the reads analytics makes.
import { numberValue } from "../market"
import { bytes, json, link, request, sleep, tokens, type Media, type Provider, type Send } from "./net"

const api = "https://api.x.com/2"

export const x: Provider = {
  env: ["X_CLIENT_ID", "X_CLIENT_SECRET"],
  pkce: true,
  authorize: (input) =>
    link("https://x.com/i/oauth2/authorize", {
      response_type: "code",
      client_id: input.client.id,
      redirect_uri: input.redirect,
      scope: "tweet.read tweet.write users.read media.write offline.access",
      state: input.state,
      code_challenge: input.challenge,
      code_challenge_method: "S256",
    }),
  exchange: async (input) =>
    tokens(
      await json(`${api}/oauth2/token`, {
        method: "POST",
        basic: [input.client.id, input.client.secret],
        form: { code: input.code, grant_type: "authorization_code", redirect_uri: input.redirect, code_verifier: input.verifier ?? "" },
      }),
    ),
  // X rotates refresh tokens: each refresh returns a new one.
  refresh: async (current, client) =>
    tokens(
      await json(`${api}/oauth2/token`, {
        method: "POST",
        basic: [client.id, client.secret],
        form: { grant_type: "refresh_token", refresh_token: current.refresh ?? "" },
      }),
    ),
  profile: async (current) => {
    const user = (await json(`${api}/users/me?user.fields=profile_image_url`, { token: current.access })).data
    return { id: String(user.id), name: `@${user.username}`, avatar: user.profile_image_url }
  },
  publish: async (job, send) => {
    const ids: string[] = []
    for (const media of job.media) ids.push(await upload(job.tokens.access, media, send))
    const post = await (
      await send(`${api}/tweets`, { method: "POST", token: job.tokens.access, json: { text: job.content, ...(ids.length ? { media: { media_ids: ids } } : {}) } }, true)
    ).json()
    return { id: String(post.data.id), url: `https://x.com/i/web/status/${post.data.id}` }
  },
  metrics: async (input) => {
    const me = (await json(`${api}/users/me?user.fields=public_metrics`, { token: input.tokens.access })).data?.public_metrics ?? {}
    const followers = numberValue(me.followers_count)
    const snapshot = followers === null ? [] : [{ key: "followers_count", value: followers, window: "snapshot" as const }]
    const ids = input.published.filter((id) => /^\d{1,19}$/.test(id)).slice(0, 100)
    if (!ids.length) return { status: "partial", note: "No posts published through Cryzo in this period yet.", metrics: snapshot, series: [] }
    const rows: any[] = (await json(`${api}/tweets?ids=${ids.join(",")}&tweet.fields=public_metrics`, { token: input.tokens.access })).data ?? []
    const sum = (key: string) => rows.reduce((total, row) => total + (numberValue(row.public_metrics?.[key]) ?? 0), 0)
    return {
      status: "partial",
      note: "Lifetime public counters for posts published through Cryzo.",
      metrics: [
        ...snapshot,
        ...["impression_count", "like_count", "reply_count", "retweet_count", "quote_count", "bookmark_count"].map((key) => ({
          key,
          value: sum(key),
          window: "lifetime" as const,
        })),
      ],
      series: [],
    }
  },
}

async function upload(token: string, media: Media, send: Send) {
  const category = media.type.startsWith("video/") ? "tweet_video" : media.type === "image/gif" ? "tweet_gif" : "tweet_image"
  const started = await (
    await send(`${api}/media/upload/initialize`, { method: "POST", token, json: { media_type: media.type, total_bytes: media.size, media_category: category } })
  ).json()
  const id = String(started.data.id)
  // X accepts segments of up to 5 MB.
  const segment = 4 * 1024 * 1024
  for (let index = 0; index * segment < media.size; index++) {
    const form = new FormData()
    form.set("media", new Blob([await bytes(media.url, index * segment, Math.min((index + 1) * segment, media.size) - 1)]))
    form.set("segment_index", String(index))
    await send(`${api}/media/upload/${id}/append`, { method: "POST", token, body: form, signal: AbortSignal.timeout(120_000) })
  }
  let info = (await (await send(`${api}/media/upload/${id}/finalize`, { method: "POST", token })).json()).data?.processing_info
  for (let attempt = 0; info && info.state !== "succeeded"; attempt++) {
    if (info.state === "failed" || attempt >= 40) throw new Error("X could not process the media file.")
    await sleep(Math.min(10, Number(info.check_after_secs) || 2) * 1000)
    info = (await (await request(`${api}/media/upload?command=STATUS&media_id=${id}`, { token })).json()).data?.processing_info
  }
  return id
}
