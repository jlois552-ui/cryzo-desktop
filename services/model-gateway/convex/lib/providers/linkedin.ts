// LinkedIn: personal posting through "Share on LinkedIn" works for any member. Company pages
// need the Community Management API, so their scopes are requested only once MARKET_PENDING
// no longer lists "linkedin-page".
import { numberValue } from "../market"
import { bytes, json, link, pending, request, tokens, type Choice, type Media, type Provider, type Send } from "./net"

const api = "https://api.linkedin.com/rest"
// Versioned API: LinkedIn supports each monthly version for a year.
const version = "202603"
const rest = (token: string) => ({ token, headers: { "LinkedIn-Version": version, "X-Restli-Protocol-Version": "2.0.0" } })

export const linkedin: Provider = {
  env: ["LINKEDIN_CLIENT_ID", "LINKEDIN_CLIENT_SECRET"],
  pkce: false,
  authorize: (input) =>
    link("https://www.linkedin.com/oauth/v2/authorization", {
      response_type: "code",
      client_id: input.client.id,
      redirect_uri: input.redirect,
      state: input.state,
      scope: ["openid profile w_member_social", ...(pages() ? ["r_organization_social w_organization_social rw_organization_admin"] : [])].join(" "),
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
        form: { grant_type: "refresh_token", refresh_token: current.refresh ?? "", client_id: client.id, client_secret: client.secret },
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
    const acls = await json(`${api}/organizationAcls?q=roleAssignee&role=ADMINISTRATOR&state=APPROVED`, rest(current.access))
    for (const urn of (acls.elements ?? []).map((item: any) => String(item.organization ?? "")).filter(Boolean).slice(0, 50)) {
      const organization = await json(`${api}/organizations/${urn.split(":").at(-1)}`, rest(current.access)).catch(() => null)
      choices.push({ id: urn, name: String(organization?.localizedName ?? urn) })
    }
    return choices
  },
  publish: async (job, send) => {
    const owner = job.targetId!
    const token = job.tokens.access
    const media = []
    for (const item of job.media) media.push(item.type.startsWith("video/") ? await video(token, owner, item, send) : await image(token, owner, item, send))
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
          ...(media.length === 1 ? { content: { media: { id: media[0] } } } : media.length ? { content: { multiImage: { images: media.map((id) => ({ id })) } } } : {}),
        },
      },
      true,
    )
    const id = response.headers.get("x-restli-id") ?? ""
    if (!id) throw new Error("LinkedIn returned no post ID. Check LinkedIn before retrying.")
    return { id, url: `https://www.linkedin.com/feed/update/${id}` }
  },
  metrics: async (input) => {
    if (!input.targetId?.startsWith("urn:li:organization:"))
      return { status: "unsupported", note: "LinkedIn shares analytics for company pages only.", metrics: [], series: [] }
    const urn = encodeURIComponent(input.targetId)
    const size = await json(`${api}/networkSizes/${urn}?edgeType=COMPANY_FOLLOWED_BY_MEMBER`, rest(input.tokens.access))
    const shares = await json(`${api}/organizationalEntityShareStatistics?q=organizationalEntity&organizationalEntity=${urn}`, rest(input.tokens.access))
    const totals = shares.elements?.[0]?.totalShareStatistics ?? {}
    const followers = numberValue(size.firstDegreeSize)
    return {
      status: "partial",
      note: "Lifetime page totals.",
      metrics: [
        ...(followers === null ? [] : [{ key: "followers_count", value: followers, window: "snapshot" as const }]),
        ...["impressionCount", "clickCount", "likeCount", "commentCount", "shareCount"].flatMap((key) => {
          const value = numberValue(totals[key])
          return value === null ? [] : [{ key, value, window: "lifetime" as const }]
        }),
      ],
      series: [],
    }
  },
}

function pages() {
  return !pending("linkedin-page")
}

// Commentary uses LinkedIn's "little text" format, where these characters are reserved.
function escape(text: string) {
  return text.replace(/[\\|{}@[\]()<>#*_~]/g, (char) => `\\${char}`)
}

async function image(token: string, owner: string, media: Media, send: Send) {
  const started = await (await send(`${api}/images?action=initializeUpload`, { method: "POST", ...rest(token), json: { initializeUploadRequest: { owner } } })).json()
  await send(started.value.uploadUrl, { method: "PUT", token, body: await bytes(media.url, 0, media.size - 1), signal: AbortSignal.timeout(120_000) })
  return String(started.value.image)
}

async function video(token: string, owner: string, media: Media, send: Send) {
  const started = await (
    await send(`${api}/videos?action=initializeUpload`, {
      method: "POST",
      ...rest(token),
      json: { initializeUploadRequest: { owner, fileSizeBytes: media.size, uploadCaptions: false, uploadThumbnail: false } },
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
    json: { finalizeUploadRequest: { video: started.value.video, uploadToken: started.value.uploadToken ?? "", uploadedPartIds: parts } },
  })
  return String(started.value.video)
}
