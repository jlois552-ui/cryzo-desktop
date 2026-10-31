// Facebook Pages and Instagram professional accounts through one Meta app and Facebook Login.
// Long-lived user tokens last about 60 days and cannot be refreshed; Page tokens derived from
// them do not expire, so posting keeps working until the user's token lapses.
import { graphMetrics, numberValue } from "../market"
import { json, link, sleep, type Choice, type Init, type Provider } from "./net"

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
    targets: async (current) => {
      const pages: any[] = []
      let next: string | undefined = `${graph}/me/accounts?fields=id,name,access_token,instagram_business_account{id,username}&limit=100`
      for (let page = 0; next && page < 20; page++) {
        const data: any = await json(next, page ? {} : { token: current.access })
        pages.push(...(data.data ?? []))
        next = data.paging?.next
      }
      return choose(pages)
    },
  }
}

export const facebook: Provider = {
  ...meta("pages_show_list,pages_manage_posts,pages_read_engagement,read_insights,business_management", (pages) =>
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

export const instagram: Provider = {
  ...meta("instagram_basic,instagram_content_publish,instagram_manage_insights,pages_show_list,pages_read_engagement,business_management", (pages) =>
    pages
      .filter((page) => page.instagram_business_account?.id)
      .map((page) => ({
        id: String(page.instagram_business_account.id),
        name: `@${page.instagram_business_account.username ?? page.name}`,
        token: page.access_token,
      })),
  ),
  publish: async (job, send) => {
    const token = job.targetToken!
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
    if (!input.targetId || !input.targetToken)
      return { status: "target_required", note: "Choose an Instagram Business or Creator account.", metrics: [], series: [] }
    const auth: Init = { token: input.targetToken }
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

