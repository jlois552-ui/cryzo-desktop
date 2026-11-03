// LinkedIn: personal posting through "Share on LinkedIn" works for any member. Company pages
// need the Community Management API, so their scopes are requested only once MARKET_PENDING
// no longer lists "linkedin-page".
import { numberValue } from "../market"
import {
  bytes,
  json,
  link,
  pending,
  request,
  tokens,
  type Choice,
  type Media,
  type Provider,
  type Send,
  type Measure,
  type Measured,
  ProviderError,
} from "./net"

const api = "https://api.linkedin.com/rest"
// Versioned API: LinkedIn supports each monthly version for a year.
const version = "202610"
const rest = (token: string) => ({
  token,
  headers: { "LinkedIn-Version": version, "X-Restli-Protocol-Version": "2.0.0" },
})

export const linkedin: Provider = {
  env: ["LINKEDIN_CLIENT_ID", "LINKEDIN_CLIENT_SECRET"],
  pkce: false,
  authorize: (input) =>
    link("https://www.linkedin.com/oauth/v2/authorization", {
      response_type: "code",
      client_id: input.client.id,
      redirect_uri: input.redirect,
      state: input.state,
      scope: [
        "openid profile w_member_social",
        ...(memberAnalytics() ? ["r_member_postAnalytics"] : []),
        ...(pages() ? ["r_organization_social w_organization_social rw_organization_admin"] : []),
      ].join(" "),
    }),
  exchange: async (input) =>
    tokens(
      await json("https://www.linkedin.com/oauth/v2/accessToken", {
        method: "POST",
        form: {
          grant_type: "authorization_code",
          code: input.code,
          redirect_uri: input.redirect,
          client_id: input.client.id,
          client_secret: input.client.secret,
        },
      }),
    ),
  // Refresh tokens are issued only to approved partner apps; others reconnect every 60 days.
  refresh: async (current, client) =>
    tokens(
      await json("https://www.linkedin.com/oauth/v2/accessToken", {
        method: "POST",
        form: {
          grant_type: "refresh_token",
          refresh_token: current.refresh ?? "",
          client_id: client.id,
          client_secret: client.secret,
        },
      }),
      current,
    ),
  profile: async (current) => {
    const user = await json("https://api.linkedin.com/v2/userinfo", { token: current.access })
    const name = String(user.name ?? "LinkedIn member")
    return { id: String(user.sub), name, avatar: user.picture, target: { id: `urn:li:person:${user.sub}`, name } }
  },
  targets: async (current) => {
    const me = await linkedin.profile(current)
    const choices: Choice[] = [me.target!]
    if (!pages()) return choices
    const acls = await json(
      `${api}/organizationAcls?q=roleAssignee&role=ADMINISTRATOR&state=APPROVED`,
      rest(current.access),
    )
    for (const urn of (acls.elements ?? [])
      .map((item: any) => String(item.organization ?? ""))
      .filter(Boolean)
      .slice(0, 50)) {
      const organization = await json(`${api}/organizations/${urn.split(":").at(-1)}`, rest(current.access)).catch(
        () => null,
      )
      choices.push({ id: urn, name: String(organization?.localizedName ?? urn) })
    }
    return choices
  },
  publish: async (job, send) => {
    const owner = job.targetId!
    const token = job.tokens.access
    const media = []
    for (const item of job.media)
      media.push(
        item.type.startsWith("video/") ? await video(token, owner, item, send) : await image(token, owner, item, send),
      )
    const response = await send(
      `${api}/posts`,
      {
        method: "POST",
        ...rest(token),
        json: {
          author: owner,
          commentary: escape(job.content),
          visibility: "PUBLIC",
          distribution: { feedDistribution: "MAIN_FEED", targetEntities: [], thirdPartyDistributionChannels: [] },
          lifecycleState: "PUBLISHED",
          isReshareDisabledByAuthor: false,
          ...(media.length === 1
            ? { content: { media: { id: media[0] } } }
            : media.length
              ? { content: { multiImage: { images: media.map((id) => ({ id })) } } }
              : {}),
        },
      },
      true,
    )
    const id = response.headers.get("x-restli-id") ?? ""
    if (!id) throw new Error("LinkedIn returned no post ID. Check LinkedIn before retrying.")
    return { id, url: `https://www.linkedin.com/feed/update/${id}` }
  },
  metrics: async (input) => {
    const organization = input.targetId?.startsWith("urn:li:organization:")
    if ((organization && !pages()) || (!organization && !memberAnalytics()))
      return {
        status: "permission_required",
        permissionReason: "approval_pending",
        note: "LinkedIn analytics requires Community Management approval for this account type.",
        metrics: [],
        series: [],
      }
    if (!input.targetId)
      return { status: "target_required", note: "Choose a LinkedIn profile or Page.", metrics: [], series: [] }
    const scopes = input.tokens.scope?.split(/\s+/)
    const required = organization ? "rw_organization_admin" : "r_member_postAnalytics"
    if (scopes && !scopes.includes(required))
      return {
        status: "permission_required",
        permissionReason: "reconnect_required",
        note: "Reconnect LinkedIn to grant analytics access.",
        metrics: [],
        series: [],
      }
    try {
      return await (organization ? pageMetrics(input) : memberMetrics(input))
    } catch (error) {
      if (error instanceof ProviderError && (error.status === 401 || error.status === 403))
        return {
          status: "permission_required",
          permissionReason: error.status === 401 ? "reconnect_required" : "approval_pending",
          note:
            error.status === 401
              ? "Reconnect LinkedIn to restore access."
              : "LinkedIn denied analytics access. Check app approval and Page administrator access.",
          metrics: [],
          series: [],
        }
      throw error
    }
  },
}

function memberAnalytics() {
  return process.env.LINKEDIN_MEMBER_ANALYTICS_ENABLED === "true"
}

function pages() {
  return !pending("linkedin-page")
}

// Commentary uses LinkedIn's "little text" format, where these characters are reserved.
function escape(text: string) {
  return text.replace(/[\\|{}@[\]()<>#*_~]/g, (char) => `\\${char}`)
}

async function image(token: string, owner: string, media: Media, send: Send) {
  const started = await (
    await send(`${api}/images?action=initializeUpload`, {
      method: "POST",
      ...rest(token),
      json: { initializeUploadRequest: { owner } },
    })
  ).json()
  await send(started.value.uploadUrl, {
    method: "PUT",
    token,
    body: await bytes(media.url, 0, media.size - 1),
    signal: AbortSignal.timeout(120_000),
  })
  return String(started.value.image)
}

async function video(token: string, owner: string, media: Media, send: Send) {
  const started = await (
    await send(`${api}/videos?action=initializeUpload`, {
      method: "POST",
      ...rest(token),
      json: {
        initializeUploadRequest: { owner, fileSizeBytes: media.size, uploadCaptions: false, uploadThumbnail: false },
      },
    })
  ).json()
  const parts: string[] = []
  for (const part of started.value.uploadInstructions ?? []) {
    const response = await send(part.uploadUrl, {
      method: "PUT",
      body: await bytes(media.url, Number(part.firstByte), Number(part.lastByte)),
      signal: AbortSignal.timeout(120_000),
    })
    parts.push(response.headers.get("etag") ?? "")
  }
  await request(`${api}/videos?action=finalizeUpload`, {
    method: "POST",
    ...rest(token),
    json: {
      finalizeUploadRequest: {
        video: started.value.video,
        uploadToken: started.value.uploadToken ?? "",
        uploadedPartIds: parts,
      },
    },
  })
  return String(started.value.video)
}

// LinkedIn date ranges are calendar dates with an exclusive end; include today's UTC date.
function dates(input: Measure) {
  const end = new Date(input.to)
  end.setUTCHours(0, 0, 0, 0)
  end.setUTCDate(end.getUTCDate() + 1)
  const date = (value: Date) =>
    `(year:${value.getUTCFullYear()},month:${value.getUTCMonth() + 1},day:${value.getUTCDate()})`
  return { end, range: `(start:${date(input.from)},end:${date(end)})` }
}

async function measured(parts: Promise<Measured>[]): Promise<Measured> {
  const results = await Promise.allSettled(parts)
  const good = results.flatMap((result) => (result.status === "fulfilled" ? [result.value] : []))
  const bad = results.flatMap((result) => (result.status === "rejected" ? [result.reason as Error] : []))
  if (!good.length && bad.length) throw bad[0]
  return {
    status: bad.length || good.some((result) => result.status === "partial") ? "partial" : "ready",
    metrics: good.flatMap((result) => result.metrics),
    series: good.flatMap((result) => result.series),
    ...(bad.length || good.some((result) => result.status === "partial")
      ? { note: "Some LinkedIn metrics are unavailable. Available values are shown." }
      : {}),
  }
}

async function memberMetrics(input: Measure): Promise<Measured> {
  const range = dates(input).range
  return measured(
    ["IMPRESSION", "MEMBERS_REACHED", "REACTION", "COMMENT", "RESHARE"].map(async (queryType): Promise<Measured> => {
      const load = async (aggregation: string) => {
        const result = await json(
          `${api}/memberCreatorPostAnalytics?q=me&queryType=${queryType}&aggregation=${aggregation}&dateRange=${range}`,
          rest(input.tokens.access),
        )
        return (result.elements ?? []) as {
          count?: number
          dateRange?: { start?: { year: number; month: number; day: number } }
        }[]
      }
      const totals = await load("TOTAL")
      const values = totals.flatMap((item) => {
        const value = numberValue(item.count)
        return value === null ? [] : [value]
      })
      const value = values.reduce((sum, item) => sum + item, 0)
      const daily = queryType === "MEMBERS_REACHED" ? [] : await load("DAILY").catch(() => null)
      return {
        status: daily === null ? "partial" : "ready",
        metrics: values.length ? [{ key: queryType.toLowerCase(), value, window: "period" as const }] : [],
        series: daily?.length
          ? [
              {
                key: queryType.toLowerCase(),
                points: daily.flatMap((item) => {
                  const start = item.dateRange?.start
                  const count = numberValue(item.count)
                  return start && count !== null
                    ? [
                        {
                          date: `${start.year}-${String(start.month).padStart(2, "0")}-${String(start.day).padStart(2, "0")}`,
                          value: count,
                        },
                      ]
                    : []
                }),
              },
            ]
          : [],
      }
    }),
  )
}

async function pageMetrics(input: Measure): Promise<Measured> {
  const urn = encodeURIComponent(input.targetId!)
  const end = dates(input).end.getTime()
  return measured([
    (async () => {
      const size = await json(
        `${api}/networkSizes/${urn}?edgeType=COMPANY_FOLLOWED_BY_MEMBER`,
        rest(input.tokens.access),
      )
      const value = numberValue(size.firstDegreeSize)
      return {
        metrics: value === null ? [] : [{ key: "followers_count", value, window: "snapshot" as const }],
        series: [],
      }
    })(),
    (async () => {
      const shares = await json(
        `${api}/organizationalEntityShareStatistics?q=organizationalEntity&organizationalEntity=${urn}&timeIntervals=(timeGranularityType:DAY,timeRange:(start:${input.from.getTime()},end:${end}))`,
        rest(input.tokens.access),
      )
      const elements = (shares.elements ?? []) as {
        timeRange?: { start: number }
        totalShareStatistics?: Record<string, number>
      }[]
      const keys = ["impressionCount", "clickCount", "likeCount", "commentCount", "shareCount"]
      return {
        metrics: keys.flatMap((key) => {
          const values = elements.flatMap((item) => {
            const value = numberValue(item.totalShareStatistics?.[key])
            return value === null ? [] : [value]
          })
          return values.length
            ? [{ key, value: values.reduce((sum, value) => sum + value, 0), window: "period" as const }]
            : []
        }),
        series: keys
          .map((key) => ({
            key,
            points: elements.flatMap((item) => {
              const value = numberValue(item.totalShareStatistics?.[key])
              return item.timeRange && value !== null
                ? [{ date: new Date(item.timeRange.start).toISOString().slice(0, 10), value }]
                : []
            }),
          }))
          .filter((series) => series.points.length),
      }
    })(),
  ])
}
