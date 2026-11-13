// TikTok: Login Kit and the Content Posting API, by file upload. Direct Post publishes from Cryzo; until
// TikTok audits the app, those posts are private and at most five accounts can post per day. Drafts go to
// the creator's TikTok inbox, where they finish and post in the app, with at most five waiting per day.
import { numberValue } from "../market"
import { bytes, json, link, ProviderError, tokens, type Creator, type Init, type Job, type Provider, type Tokens } from "./net"

const api = "https://open.tiktokapis.com/v2"
// TikTok rejects URLSearchParams' automatic charset suffix on OAuth token requests.
const tokenHeaders = { "Content-Type": "application/x-www-form-urlencoded" }

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
      // Without this, an account that already approved Cryzo skips the consent page when reconnecting.
      disable_auto_auth: "1",
    }),
  exchange: async (input) =>
    tokens(
      await json(`${api}/oauth/token/`, {
        method: "POST",
        headers: tokenHeaders,
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
        headers: tokenHeaders,
        form: { client_key: client.id, client_secret: client.secret, grant_type: "refresh_token", refresh_token: current.refresh ?? "" },
      }),
      current,
    ),
  profile: async (current) => {
    const user = (await call(`${api}/user/info/?fields=open_id,display_name,avatar_url`, { token: current.access })).user
    return { id: String(user.open_id), name: String(user.display_name ?? "TikTok account"), avatar: user.avatar_url }
  },
  creator,
  publish: async (job, send) => {
    const token = job.tokens.access
    const video = job.media[0]
    const draft = job.options.tiktokMode === "draft"
    // One chunk up to 64 MB; larger files use 10 MB chunks, with the remainder in the last one.
    const chunk = video.size <= 64 * 1024 * 1024 ? video.size : 10 * 1024 * 1024
    const count = Math.max(1, Math.floor(video.size / chunk))
    const source_info = { source: "FILE_UPLOAD", video_size: video.size, chunk_size: chunk, total_chunk_count: count }
    const started = await call(
      `${api}/post/publish/${draft ? "inbox/video" : "video"}/init/`,
      { method: "POST", token, json: draft ? { source_info } : { post_info: await postInfo(job), source_info } },
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
    // A draft is delivered once it reaches the inbox; posting it is the creator's step in TikTok.
    if (data.status === "SEND_TO_USER_INBOX") return { state: "published" }
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

async function postInfo(job: Job) {
  const options = job.options
  const info = await creator(job.tokens)
  const privacy = options.tiktokPrivacy
  if (!privacy || !info.privacy.includes(privacy))
    throw new ProviderError(400, `TikTok allows ${info.privacy.join(", ") || "no visibility options"} for this account right now.`, true)
  return {
    title: job.content.slice(0, 2200),
    privacy_level: privacy,
    // Interactions stay off unless the creator turned them on and their account allows them.
    disable_comment: !options.tiktokComment || info.commentDisabled,
    disable_duet: !options.tiktokDuet || info.duetDisabled,
    disable_stitch: !options.tiktokStitch || info.stitchDisabled,
    brand_content_toggle: Boolean(options.tiktokDisclose && options.tiktokBranded),
    brand_organic_toggle: Boolean(options.tiktokDisclose && options.tiktokYourBrand),
  }
}

async function creator(current: Tokens): Promise<Creator> {
  const info = await call(`${api}/post/publish/creator_info/query/`, { method: "POST", token: current.access, json: {} })
  return {
    name: String(info.creator_nickname ?? info.creator_username ?? "TikTok account"),
    privacy: info.privacy_level_options ?? [],
    commentDisabled: Boolean(info.comment_disabled),
    duetDisabled: Boolean(info.duet_disabled),
    stitchDisabled: Boolean(info.stitch_disabled),
    maxDuration: numberValue(info.max_video_post_duration_sec) ?? 60,
  }
}

/** TikTok wraps results in `data` and reports errors in `error.code`, sometimes with HTTP 200. */
async function call(url: string, init: Init, send?: (url: string, init: Init, publishes: boolean) => Promise<Response>) {
  const body = send ? await (await send(url, init, true)).json() : await json(url, init)
  if (body.error?.code && body.error.code !== "ok") throw new ProviderError(400, `TikTok: ${body.error.message || body.error.code}`, true)
  return body.data ?? {}
}
