// Facebook Pages through Facebook Login, and Instagram professional accounts through Unified Login, which
// needs no Facebook Page. Facebook's long-lived user tokens last about 60 days and cannot be refreshed;
// Page tokens derived from them do not expire, so posting keeps working until the user's token lapses.
import { graphMetrics, numberValue } from "../market"
import { json, link, sleep, type Choice, type Provider } from "./net"

const version = "v25.0"
const graph = `https://graph.facebook.com/${version}`

function meta(scope: string, choose: (pages: any[]) => Choice[]): Omit<Provider, "publish" | "metrics"> {
  return {
    env: ["META_APP_ID", "META_APP_SECRET"],
    pkce: false,
    authorize: (input) =>
      link(`https://www.facebook.com/${version}/dialog/oauth`, {
        client_id: input.client.id,
        redirect_uri: input.redirect,
        state: input.state,
        response_type: "code",
        scope,
      }),
    exchange: async (input) => {
      const short = await json(
        link(`${graph}/oauth/access_token`, { client_id: input.client.id, client_secret: input.client.secret, redirect_uri: input.redirect, code: input.code }),
      )
      const long = await json(
        link(`${graph}/oauth/access_token`, {
          grant_type: "fb_exchange_token",
          client_id: input.client.id,
          client_secret: input.client.secret,
          fb_exchange_token: short.access_token,
        }),
      )
      return { access: long.access_token, expiresAt: Date.now() + (Number(long.expires_in) || 60 * 86_400) * 1000 }
    },
    profile: async (current) => {
      const me = await json(`${graph}/me?fields=id,name,picture`, { token: current.access })
      return { id: String(me.id), name: String(me.name ?? "Meta account"), avatar: me.picture?.data?.url }
    },
    // Only Pages the person administers directly. Pages reached only through a business portfolio would
    // need business_management, which Cryzo leaves out to keep Meta's review to posting and analytics.
    targets: async (current) => {
      const pages: any[] = []
      let next: string | undefined = `${graph}/me/accounts?fields=id,name,access_token,instagram_business_account{id,username}&limit=100`
      for (let page = 0; next && page < 20; page++) {
        // Paging links already carry the access token.
        const data: any = await json(next, page ? {} : { token: current.access })
        pages.push(...(data.data ?? []))
        next = data.paging?.next
      }
      return choose(pages)
    },
  }
}

export const facebook: Provider = {
  ...meta("pages_show_list,pages_manage_posts,pages_read_engagement,read_insights", (pages) =>
    pages.map((page) => ({ id: String(page.id), name: String(page.name), token: page.access_token })),
  ),
  publish: async (job, send) => {
    const token = job.targetToken!
    const video = job.media.find((item) => item.type.startsWith("video/"))
    if (video) {
      const posted = await (await send(`${graph}/${job.targetId}/videos`, { method: "POST", token, form: { file_url: video.url, description: job.content } }, true)).json()
      return { id: String(posted.id), url: `https://www.facebook.com/${posted.id}` }
    }
    // Photos are uploaded unpublished, then attached to one feed post.
    const photos: string[] = []
    for (const item of job.media)
      photos.push(String((await (await send(`${graph}/${job.targetId}/photos`, { method: "POST", token, form: { url: item.url, published: "false" } })).json()).id))
    const posted = await (
      await send(
        `${graph}/${job.targetId}/feed`,
        {
          method: "POST",
          token,
          form: { message: job.content, ...(photos.length ? { attached_media: JSON.stringify(photos.map((id) => ({ media_fbid: id }))) } : {}) },
        },
        true,
      )
    ).json()
    return { id: String(posted.id), url: `https://www.facebook.com/${posted.id}` }
  },
  metrics: async (input) => {
    if (!input.targetId || !input.targetToken) return { status: "target_required", note: "Choose the Facebook Page to report on.", metrics: [], series: [] }
    return graphMetrics(
      await json(
        link(`${graph}/${input.targetId}/insights`, {
          metric: "page_daily_follows,page_media_view,page_total_media_view_unique,page_post_engagements",
          period: "day",
          since: String(Math.floor(input.from.getTime() / 1000)),
          until: String(Math.floor(input.to.getTime() / 1000)),
        }),
        { token: input.targetToken },
      ),
    )
  },
}

// Instagram API with Unified Login: creators log in with Instagram (or Facebook) and share a professional
// account from their business portfolio; no Facebook Page is needed. Facebook Login for Business returns a
// system-user token scoped to the shared accounts. The permissions come from the dashboard configuration
// named by META_INSTAGRAM_CONFIG_ID, not from a scope parameter.
export const instagram: Provider = {
  env: ["META_APP_ID", "META_APP_SECRET"],
  pkce: false,
  authorize: (input) =>
    link(`https://www.facebook.com/${version}/dialog/oauth`, {
      client_id: input.client.id,
      config_id: process.env.META_INSTAGRAM_CONFIG_ID?.trim(),
      redirect_uri: input.redirect,
      response_type: "code",
      override_default_response_type: "true",
      state: input.state,
      // Always offers both "Log in with Instagram" and Facebook, even when Facebook is already signed in.
      prompt: "login",
    }),
  exchange: async (input) => {
    const data = await json(
      link(`${graph}/oauth/access_token`, { client_id: input.client.id, client_secret: input.client.secret, redirect_uri: input.redirect, code: input.code }),
    )
    // The configuration sets the token's lifetime; "never" returns no expires_in.
    return { access: data.access_token, ...(Number(data.expires_in) ? { expiresAt: Date.now() + Number(data.expires_in) * 1000 } : {}) }
  },
  profile: async (current) => {
    const accounts = await assigned(current.access)
    if (!accounts.length) throw new Error("Share an Instagram Business or Creator account with Cryzo when you connect.")
    // The token belongs to a system user, so the connection is named after its first Instagram account.
    const [first] = accounts
    return { id: first.id, name: first.name, avatar: first.avatar, ...(accounts.length === 1 ? { target: { id: first.id, name: first.name } } : {}) }
  },
  targets: (current) => assigned(current.access),
  publish: async (job, send) => {
    const token = job.tokens.access
    const kind = job.options.instagramPostType ?? "post"
    const create = async (form: Record<string, string>) =>
      String((await (await send(`${graph}/${job.targetId}/media`, { method: "POST", token, form })).json()).id)
    const source = (item: { url: string; type: string }): Record<string, string> =>
      item.type.startsWith("video/") ? { video_url: item.url } : { image_url: item.url }
    let container: string
    if (kind === "carousel") {
      const children: string[] = []
      for (const item of job.media) {
        const child = await create({ is_carousel_item: "true", ...source(item), ...(item.type.startsWith("video/") ? { media_type: "VIDEO" } : {}) })
        await ready(child, token)
        children.push(child)
      }
      container = await create({ media_type: "CAROUSEL", caption: job.content, children: children.join(",") })
    } else {
      const item = job.media[0]
      const video = item.type.startsWith("video/")
      container = await create({
        ...source(item),
        ...(kind === "story" ? { media_type: "STORIES" } : video ? { media_type: "REELS" } : {}),
        ...(kind === "story" ? {} : { caption: job.content }),
      })
    }
    await ready(container, token)
    const posted = await (await send(`${graph}/${job.targetId}/media_publish`, { method: "POST", token, form: { creation_id: container } }, true)).json()
    const permalink = await json(`${graph}/${posted.id}?fields=permalink`, { token }).catch(() => null)
    return { id: String(posted.id), ...(permalink?.permalink ? { url: String(permalink.permalink) } : {}) }
  },
  metrics: async (input) => {
    if (!input.targetId) return { status: "target_required", note: "Choose an Instagram Business or Creator account.", metrics: [], series: [] }
    const auth = { token: input.tokens.access }
    // Instagram reports at most the last 30 days.
    const since = String(Math.floor(Math.max(input.from.getTime(), input.to.getTime() - 30 * 86_400_000) / 1000))
    const until = String(Math.floor(input.to.getTime() / 1000))
    const insights = (metric: string, type: string) =>
      json(link(`${graph}/${input.targetId}/insights`, { metric, metric_type: type, period: "day", since, until }), auth)
    const totals = graphMetrics(await insights("views,likes,comments,shares,saves", "total_value"))
    const reach = graphMetrics(await insights("reach", "time_series"))
    const followers = numberValue((await json(`${graph}/${input.targetId}?fields=followers_count`, auth)).followers_count)
    return {
      status: input.days > 30 ? "partial" : "ready",
      metrics: [...(followers === null ? [] : [{ key: "followers_count", value: followers, window: "snapshot" as const }]), ...totals.metrics, ...reach.metrics],
      series: reach.series,
      ...(input.days > 30 ? { note: "Instagram reports at most the last 30 days." } : {}),
    }
  },
}

/** Instagram accounts the creator shared with Cryzo when they logged in. */
async function assigned(token: string): Promise<(Choice & { avatar?: string })[]> {
  const shared: any[] = (await json(`${graph}/me/assigned_instagram_accounts?limit=100`, { token })).data ?? []
  return Promise.all(
    shared.map(async (account) => {
      const id = String(account.id)
      const details = await json(`${graph}/${id}?fields=username,profile_picture_url`, { token }).catch(() => ({}))
      return { id, name: details.username ? `@${details.username}` : "Instagram account", avatar: details.profile_picture_url }
    }),
  )
}

/** Waits for Instagram to finish processing a media container (videos can take minutes). */
async function ready(container: string, token: string) {
  for (let attempt = 0; attempt < 48; attempt++) {
    const status = String((await json(`${graph}/${container}?fields=status_code`, { token })).status_code ?? "")
    if (status === "FINISHED") return
    if (status === "ERROR" || status === "EXPIRED") throw new Error("Instagram could not process the media file.")
    await sleep(5000)
  }
  throw new Error("Instagram is still processing the media. Try again in a few minutes.")
}

