// Marketing rules shared by the HTTP API, the publishing worker and tests.
// Ported from cryzo.me's social publishing (Imma2013/cryzo-v11), limited to the six supported networks.

export const channels = ["x", "facebook", "instagram", "youtube", "tiktok", "reddit"] as const
export type Channel = (typeof channels)[number]

export const toolkits: Record<Channel, string> = {
  x: "twitter",
  facebook: "facebook",
  instagram: "instagram",
  youtube: "youtube",
  tiktok: "tiktok",
  reddit: "reddit",
}

const authConfigEnv: Record<Channel, string> = {
  x: "COMPOSIO_TWITTER_AUTH_CONFIG_ID",
  facebook: "COMPOSIO_FACEBOOK_AUTH_CONFIG_ID",
  instagram: "COMPOSIO_INSTAGRAM_AUTH_CONFIG_ID",
  youtube: "COMPOSIO_YOUTUBE_AUTH_CONFIG_ID",
  tiktok: "COMPOSIO_TIKTOK_AUTH_CONFIG_ID",
  reddit: "COMPOSIO_REDDIT_AUTH_CONFIG_ID",
}

// Composio manages OAuth apps for these; X and TikTok need the owner's own developer app.
const customAuth: Channel[] = ["x", "tiktok"]

export function isChannel(value: unknown): value is Channel {
  return typeof value === "string" && channels.some((channel) => channel === value)
}

export function channelForToolkit(toolkit: string) {
  return channels.find((channel) => toolkits[channel] === toolkit)
}

export function authConfig(channel: Channel) {
  const name = authConfigEnv[channel]
  return { name, id: process.env[name]?.trim() || null, custom: customAuth.includes(channel) }
}

export type Target = { channel: Channel; connectedAccountId: string; content?: string }
export type Options = {
  redditCommunity?: string
  youtubeTitle?: string
  youtubePrivacy?: string
  tiktokPrivacy?: string
  instagramPostType?: "post" | "reel" | "story" | "carousel"
}
export type Account = { channel: string; connectedAccountId: string; targetId?: string }
export type PostDraft = { content: string; targets: Target[]; options?: Options; mediaTypes: string[] }

/** The first reason a post cannot be published, or null. Drafts skip network checks. */
export function publishProblem(post: PostDraft, accounts: Account[]): string | null {
  if (!post.targets.length) return "Choose at least one channel."
  for (const target of post.targets) {
    const text = (target.content ?? post.content).trim()
    if (!text && post.mediaTypes.length === 0) return `Write the ${target.channel} post before scheduling.`
    const account = accounts.find((item) => item.channel === target.channel && item.connectedAccountId === target.connectedAccountId)
    if (!account) return `Connect and select ${target.channel} before publishing.`
    if (target.channel === "facebook" && !/^\d+$/.test(account.targetId ?? "")) return "Choose the Facebook Page to post to."
    if (target.channel === "instagram" && !/^\d+$/.test(account.targetId ?? "")) return "Choose an Instagram Business or Creator account."
    if (target.channel === "x" && text.length > 280) return "X posts must be 280 characters or fewer."
  }
  const used = new Set(post.targets.map((target) => target.channel))
  const videos = post.mediaTypes.filter((type) => type.startsWith("video/")).length
  const images = post.mediaTypes.filter((type) => type.startsWith("image/")).length
  if (post.mediaTypes.length > 10) return "Use at most ten media files."
  if (used.has("reddit") && !post.options?.redditCommunity?.trim()) return "Choose a Reddit community."
  if (used.has("reddit") && post.mediaTypes.length) return "Reddit currently supports text posts only."
  if (used.has("x") && (videos || images > 4)) return "X supports up to four images and no video."
  if (used.has("tiktok") && (videos !== 1 || post.mediaTypes.length !== 1)) return "TikTok needs exactly one video."
  if (used.has("youtube") && (videos !== 1 || images > 1 || videos + images !== post.mediaTypes.length))
    return "YouTube needs one video and at most one thumbnail image."
  if (used.has("facebook") && videos && post.mediaTypes.length !== 1) return "Facebook video posts need exactly one video."
  if (used.has("instagram")) {
    const kind = post.options?.instagramPostType ?? "post"
    if (kind === "carousel" && (post.mediaTypes.length < 2 || post.mediaTypes.length > 10)) return "Instagram carousels need 2 to 10 media files."
    if (kind !== "carousel" && post.mediaTypes.length !== 1) return "Instagram posts, reels and stories need exactly one media file."
    if (kind === "reel" && !videos) return "Instagram Reels need a video."
  }
  return null
}

/** Post status from its deliveries; "unknown" means a dispatch may have happened and must be checked. */
export function postStatus(deliveries: { status: string }[]) {
  if (deliveries.length && deliveries.every((item) => item.status === "published")) return "published" as const
  if (deliveries.some((item) => item.status === "pending" || item.status === "publishing")) return "publishing" as const
  return "failed" as const
}

// Unwrap the Composio envelope without discarding Graph paging or report rows.
export function payload(value: any): any {
  let data = value
  for (let i = 0; i < 4 && data?.data && typeof data.data === "object" && !Array.isArray(data.data) && !data.rows && !data.elements; i++) data = data.data
  return data
}

export function identifier(value: unknown, keys = ["post_id", "tweet_id", "video_id", "media_id", "id"]): string | undefined {
  const entries = (item: unknown): [string, unknown][] =>
    item && typeof item === "object" ? Object.entries(item).flatMap(([key, child]) => [[key, child] as [string, unknown], ...entries(child)]) : []
  const found = entries(value).find(([key, child]) => keys.includes(key) && (typeof child === "string" || typeof child === "number"))
  return found?.[1]?.toString()
}

export function numberValue(value: unknown): number | null {
  if (typeof value === "number") return Number.isFinite(value) ? value : null
  if (typeof value === "string" && value.trim() && Number.isFinite(Number(value))) return Number(value)
  return null
}

export type Metric = { key: string; value: number; window: "period" | "snapshot" | "lifetime" }
export type Series = { key: string; points: { date: string; value: number }[] }

/** Facebook and Instagram Graph insights: daily series plus a period total (latest value for snapshots and unique reach). */
export function graphMetrics(value: unknown): { metrics: Metric[]; series: Series[] } {
  const root = payload(value)
  const rows = Array.isArray(root?.data) ? root.data : Array.isArray(root) ? root : []
  const metrics: Metric[] = [], series: Series[] = []
  for (const row of rows) {
    const key = String(row?.name || "")
    if (!key) continue
    const points = (row.values || []).flatMap((item: any) => {
      const amount = numberValue(item.value)
      return amount === null ? [] : [{ date: String(item.end_time || item.date || "").slice(0, 10), value: amount }]
    })
    const total = numberValue(row.total_value?.value) ?? numberValue(row.value)
    const latest = /follows|followers|reach|unique/.test(key)
    const amount = total ?? (points.length ? (latest ? points.at(-1)!.value : points.reduce((sum: number, point: { value: number }) => sum + point.value, 0)) : null)
    if (amount === null) continue
    metrics.push({ key, value: amount, window: latest && total === null ? "snapshot" : "period" })
    if (points.length) series.push({ key, points })
  }
  return { metrics, series }
}

/** YouTube Analytics report rows by day. Averages are weighted by views. */
export function youtubeMetrics(value: unknown): { metrics: Metric[]; series: Series[] } {
  const data = payload(value)
  const names: string[] = (data?.columnHeaders || data?.column_headers || []).map((header: any) => String(header.name || header))
  const rows: unknown[][] = data?.rows || []
  const day = names.indexOf("day"), views = names.indexOf("views")
  const metrics: Metric[] = [], series: Series[] = []
  names.forEach((key, index) => {
    if (index === day) return
    const valid = rows.filter((row) => numberValue(row[index]) !== null)
    if (!valid.length) return
    const average = key.startsWith("average")
    const weight = average ? valid.reduce((sum, row) => sum + (numberValue(row[views]) ?? 0), 0) : 0
    if (average && !weight) return
    const amount = average
      ? valid.reduce((sum, row) => sum + numberValue(row[index])! * (numberValue(row[views]) ?? 0), 0) / weight
      : valid.reduce((sum, row) => sum + numberValue(row[index])!, 0)
    metrics.push({ key, value: amount, window: "period" })
    if (day >= 0) series.push({ key, points: valid.map((row) => ({ date: String(row[day]), value: numberValue(row[index])! })) })
  })
  return { metrics, series }
}
