// Marketing rules shared by the HTTP API, the publishing worker and tests.
// Limited to the seven supported networks; each network's API calls live in `lib/providers`.

export const channels = ["x", "linkedin", "facebook", "instagram", "youtube", "tiktok", "reddit"] as const
export type Channel = (typeof channels)[number]

export const labels: Record<Channel, string> = {
  x: "X",
  linkedin: "LinkedIn",
  facebook: "Facebook",
  instagram: "Instagram",
  youtube: "YouTube",
  tiktok: "TikTok",
  reddit: "Reddit",
}

export function isChannel(value: unknown): value is Channel {
  return typeof value === "string" && channels.some((channel) => channel === value)
}

export type Target = { channel: Channel; connectedAccountId: string; content?: string }
export type Options = {
  redditCommunity?: string
  youtubeTitle?: string
  youtubePrivacy?: string
  /** Direct Post, or send to the creator's TikTok inbox to finish in the app. Direct is the default. */
  tiktokMode?: "direct" | "draft"
  tiktokPrivacy?: string
  /** TikTok interactions the creator allows; each starts off, as TikTok's sharing guidelines require. */
  tiktokComment?: boolean
  tiktokDuet?: boolean
  tiktokStitch?: boolean
  /** Commercial content disclosure: promoting the creator's own brand, a third party, or both. */
  tiktokDisclose?: boolean
  tiktokYourBrand?: boolean
  tiktokBranded?: boolean
  instagramPostType?: "post" | "reel" | "story" | "carousel"
}
export type Account = { channel: string; connectedAccountId: string; targetId?: string; needsReconnect?: boolean }
export type PostDraft = { content: string; targets: Target[]; options?: Options; mediaTypes: string[] }

/** The first reason a post cannot be published, or null. Drafts skip network checks. */
export function publishProblem(post: PostDraft, accounts: Account[]): string | null {
  if (!post.targets.length) return "Choose at least one channel."
  for (const target of post.targets) {
    const text = (target.content ?? post.content).trim()
    // TikTok's video requirement below applies to Direct Post and inbox drafts; captions are optional.
    if (!text && post.mediaTypes.length === 0 && target.channel !== "tiktok") return `Write the ${target.channel} post before scheduling.`
    const account = accounts.find((item) => item.channel === target.channel && item.connectedAccountId === target.connectedAccountId)
    if (!account) return `Connect and select ${target.channel} before publishing.`
    if (account.needsReconnect) return `Reconnect ${labels[target.channel]} before publishing.`
    if (target.channel === "linkedin" && !account.targetId) return "Choose the LinkedIn profile or page to post to."
    if (target.channel === "facebook" && !/^\d+$/.test(account.targetId ?? "")) return "Choose the Facebook Page to post to."
    if (target.channel === "instagram" && !/^\d+$/.test(account.targetId ?? "")) return "Choose an Instagram Business or Creator account."
    if (target.channel === "x" && text.length > 280) return "X posts must be 280 characters or fewer."
    if (target.channel === "linkedin" && text.length > 3000) return "LinkedIn posts must be 3,000 characters or fewer."
  }
  const used = new Set(post.targets.map((target) => target.channel))
  const videos = post.mediaTypes.filter((type) => type.startsWith("video/")).length
  const images = post.mediaTypes.filter((type) => type.startsWith("image/")).length
  if (post.mediaTypes.length > 10) return "Use at most ten media files."
  if (used.has("reddit") && !post.options?.redditCommunity?.trim()) return "Choose a Reddit community."
  if (used.has("reddit") && post.mediaTypes.length) return "Reddit currently supports text posts only."
  if (used.has("x") && (videos > 1 || images > 4 || (videos && images))) return "X supports up to four images or one video."
  if (used.has("linkedin") && (videos > 1 || images > 9 || (videos && images))) return "LinkedIn supports up to nine images or one video."
  if (used.has("tiktok") && (videos !== 1 || post.mediaTypes.length !== 1)) return "TikTok needs exactly one video."
  if (used.has("tiktok") && post.options?.tiktokMode !== "draft") {
    const options = post.options ?? {}
    // TikTok requires the creator to pick visibility themselves; nothing is preselected.
    if (!options.tiktokPrivacy) return "Choose who can see the TikTok video."
    if (options.tiktokDisclose && !options.tiktokYourBrand && !options.tiktokBranded)
      return "Choose whether the TikTok video promotes your brand, a third party, or both."
    if (options.tiktokDisclose && options.tiktokBranded && options.tiktokPrivacy === "SELF_ONLY")
      return "Branded content on TikTok can't be private."
  }
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

// Unwrap nested `data` envelopes without discarding Graph paging or report rows.
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
