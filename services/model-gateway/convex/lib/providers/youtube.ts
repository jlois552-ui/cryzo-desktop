// YouTube: Google OAuth with offline access, resumable uploads and YouTube Analytics.
// youtube.upload is a sensitive scope: until Google verifies the app, only test users can connect.
import { youtubeMetrics } from "../market"
import { bytes, json, link, request, tokens, type Provider } from "./net"

const scope = [
  "openid",
  "https://www.googleapis.com/auth/youtube.upload",
  "https://www.googleapis.com/auth/youtube.readonly",
  "https://www.googleapis.com/auth/yt-analytics.readonly",
].join(" ")

export const youtube: Provider = {
  env: ["YOUTUBE_CLIENT_ID", "YOUTUBE_CLIENT_SECRET"],
  pkce: true,
  authorize: (input) =>
    link("https://accounts.google.com/o/oauth2/v2/auth", {
      client_id: input.client.id,
      redirect_uri: input.redirect,
      response_type: "code",
      scope,
      access_type: "offline",
      // Google returns a refresh token only on consent.
      prompt: "consent",
      state: input.state,
      code_challenge: input.challenge,
      code_challenge_method: "S256",
    }),
  exchange: async (input) =>
    tokens(
      await json("https://oauth2.googleapis.com/token", {
        method: "POST",
        form: {
          code: input.code,
          client_id: input.client.id,
          client_secret: input.client.secret,
          redirect_uri: input.redirect,
          grant_type: "authorization_code",
          code_verifier: input.verifier ?? "",
        },
      }),
    ),
  refresh: async (current, client) =>
    tokens(
      await json("https://oauth2.googleapis.com/token", {
        method: "POST",
        form: { grant_type: "refresh_token", refresh_token: current.refresh ?? "", client_id: client.id, client_secret: client.secret },
      }),
      current,
    ),
  profile: async (current) => {
    const channel = (await json("https://www.googleapis.com/youtube/v3/channels?part=snippet&mine=true", { token: current.access })).items?.[0]
    if (!channel) throw new Error("This Google account has no YouTube channel. Create one on YouTube, then connect again.")
    return { id: String(channel.id), name: String(channel.snippet?.title ?? "YouTube channel"), avatar: channel.snippet?.thumbnails?.default?.url }
  },
  publish: async (job, send) => {
    const video = job.media.find((item) => item.type.startsWith("video/"))!
    const thumbnail = job.media.find((item) => item.type.startsWith("image/"))
    // YouTube rejects angle brackets in titles and descriptions.
    const clean = (text: string) => text.replace(/[<>]/g, "")
    const session = await send("https://www.googleapis.com/upload/youtube/v3/videos?uploadType=resumable&part=snippet,status", {
      method: "POST",
      token: job.tokens.access,
      headers: { "X-Upload-Content-Type": video.type, "X-Upload-Content-Length": String(video.size) },
      json: {
        snippet: {
          title: clean(job.options.youtubeTitle?.trim() || job.content.split("\n")[0] || "Untitled").slice(0, 100),
          description: clean(job.content).slice(0, 5000),
          categoryId: "22",
        },
        status: { privacyStatus: job.options.youtubePrivacy || "public", selfDeclaredMadeForKids: false },
      },
    })
    const location = session.headers.get("location")
    if (!location) throw new Error("YouTube did not start the upload.")
    const source = await fetch(video.url, { signal: AbortSignal.timeout(280_000) })
    if (!source.ok || !source.body) throw new Error("The video file could not be read. Upload it again.")
    const uploaded = await (
      await send(
        location,
        {
          method: "PUT",
          headers: { "Content-Type": video.type, "Content-Length": String(video.size) },
          body: source.body,
          duplex: "half",
          signal: AbortSignal.timeout(280_000),
        },
        true,
      )
    ).json()
    const id = String(uploaded.id)
    // A custom thumbnail needs a verified channel; the video stands without it.
    if (thumbnail)
      await request(`https://www.googleapis.com/upload/youtube/v3/thumbnails/set?videoId=${id}`, {
        method: "POST",
        token: job.tokens.access,
        headers: { "Content-Type": thumbnail.type },
        body: await bytes(thumbnail.url, 0, thumbnail.size - 1),
      }).catch(() => null)
    return { id, url: `https://www.youtube.com/watch?v=${id}` }
  },
  metrics: async (input) =>
    youtubeMetrics(
      await json(
        link("https://youtubeanalytics.googleapis.com/v2/reports", {
          ids: "channel==MINE",
          startDate: input.from.toISOString().slice(0, 10),
          endDate: input.to.toISOString().slice(0, 10),
          metrics: "views,likes,comments,estimatedMinutesWatched,averageViewDuration,subscribersGained,subscribersLost",
          dimensions: "day",
          sort: "day",
        }),
        { token: input.tokens.access },
      ),
    ),
}
