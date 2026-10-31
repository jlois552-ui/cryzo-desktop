// TikTok: Login Kit and the Content Posting API (direct post, file upload). Until TikTok
// audits the app, every post is private and at most five accounts can post per day.
import { numberValue } from "../market"
import { bytes, json, link, ProviderError, tokens, type Init, type Provider } from "./net"

const api = "https://open.tiktokapis.com/v2"

export const tiktok: Provider = {
  env: ["TIKTOK_CLIENT_KEY", "TIKTOK_CLIENT_SECRET"],
  pkce: false,
  authorize: (input) =>
    link("https://www.tiktok.com/v2/auth/authorize/", {
      client_key: input.client.id,
      scope: "user.info.basic,user.info.stats,video.publish,video.upload",
      response_type: "code",
      redirect_uri: input.redirect,
      state: input.state,
    }),
  exchange: async (input) =>
    tokens(
      await json(`${api}/oauth/token/`, {
        method: "POST",
        form: {
          client_key: input.client.id,
          client_secret: input.client.secret,
          code: input.code,
          grant_type: "authorization_code",
          redirect_uri: input.redirect,
        },
      }),
    ),
  refresh: async (current, client) =>
    tokens(
      await json(`${api}/oauth/token/`, {
        method: "POST",
        form: { client_key: client.id, client_secret: client.secret, grant_type: "refresh_token", refresh_token: current.refresh ?? "" },
      }),
      current,
    ),
  profile: async (current) => {
    const user = (await call(`${api}/user/info/?fields=open_id,display_name,avatar_url`, { token: current.access })).user
    return { id: String(user.open_id), name: String(user.display_name ?? "TikTok account"), avatar: user.avatar_url }
  },
  publish: async (job, send) => {
    const token = job.tokens.access
    const video = job.media[0]
    const creator = await call(`${api}/post/publish/creator_info/query/`, { method: "POST", token, json: {} })
    const allowed: string[] = creator.privacy_level_options ?? []
    const privacy = job.options.tiktokPrivacy || "SELF_ONLY"
    if (!allowed.includes(privacy))
      throw new ProviderError(400, `TikTok allows ${allowed.join(", ") || "no privacy levels"} for this account right now.`, true)
    // One chunk up to 64 MB; larger files use 10 MB chunks, with the remainder in the last one.
    const chunk = video.size <= 64 * 1024 * 1024 ? video.size : 10 * 1024 * 1024
    const count = Math.max(1, Math.floor(video.size / chunk))
    const started = await call(
      `${api}/post/publish/video/init/`,
      {
        method: "POST",
        token,
        json: {
          post_info: { title: job.content.slice(0, 2200), privacy_level: privacy },
          source_info: { source: "FILE_UPLOAD", video_size: video.size, chunk_size: chunk, total_chunk_count: count },
        },
      },
      send,
    )
    for (let index = 0; index < count; index++) {
      const start = index * chunk
      const end = index === count - 1 ? video.size - 1 : start + chunk - 1
      await send(started.upload_url, {
        method: "PUT",
        headers: { "Content-Type": video.type, "Content-Range": `bytes ${start}-${end}/${video.size}` },
        body: await bytes(video.url, start, end),
        signal: AbortSignal.timeout(120_000),
      })
    }
    return { id: String(started.publish_id), pending: true }
  },
  status: async (job, remoteId) => {
    const data = await call(`${api}/post/publish/status/fetch/`, { method: "POST", token: job.tokens.access, json: { publish_id: remoteId } })
    if (data.status === "PUBLISH_COMPLETE") {
      // TikTok's field name is misspelled in its API.
      const id = data.publicaly_available_post_id?.[0]
      return { state: "published", ...(id ? { id: String(id) } : {}) }
    }
    if (data.status === "FAILED") return { state: "failed", reason: `TikTok rejected the post (${data.fail_reason ?? "no reason given"}).` }
    return { state: "processing" }
  },
  metrics: async (input) => {
    const user = (await call(`${api}/user/info/?fields=follower_count,following_count,likes_count,video_count`, { token: input.tokens.access })).user ?? {}
    return {
      status: "partial",
      note: "Account totals as of now.",
      metrics: ["follower_count", "likes_count", "video_count"].flatMap((key) => {
        const value = numberValue(user[key])
        return value === null ? [] : [{ key, value, window: "snapshot" as const }]
      }),
      series: [],
    }
  },
}

/** TikTok wraps results in `data` and reports errors in `error.code`, sometimes with HTTP 200. */
async function call(url: string, init: Init, send?: (url: string, init: Init, publishes: boolean) => Promise<Response>) {
  const body = send ? await (await send(url, init, true)).json() : await json(url, init)
  if (body.error?.code && body.error.code !== "ok") throw new ProviderError(400, `TikTok: ${body.error.message || body.error.code}`, true)
  return body.data ?? {}
}
