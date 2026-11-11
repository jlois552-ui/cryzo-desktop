import { authTokenFromCredentials } from "@/runtime/server/api"
import { useServerSDK } from "@/runtime/server/client"
import { usePlatform } from "@/runtime/platform/platform"

export const channels = ["x", "linkedin", "facebook", "instagram", "youtube", "tiktok", "reddit"] as const
export type Channel = (typeof channels)[number]

export type Group = { _id: string; name: string; color: string }
export type Account = {
  _id: string
  channel: Channel
  connectedAccountId: string
  name: string
  avatar?: string
  /** The Page, Instagram account or LinkedIn member/organization posts go to. */
  targetId?: string
  targetName?: string
  groupId?: string
  expiresAt?: number
  needsReconnect: boolean
  needsTarget: boolean
}
/** Whether a network is set up on Cryzo cloud, and whether it still waits on the platform's approval. */
export type Provider = { channel: Channel; configured: boolean; pending: boolean; pages: boolean }
export type Target = { channel: Channel; connectedAccountId: string; content?: string }
export type Options = {
  redditCommunity?: string
  youtubeTitle?: string
  youtubePrivacy?: string
  tiktokMode?: "direct" | "draft"
  tiktokPrivacy?: string
  tiktokComment?: boolean
  tiktokDuet?: boolean
  tiktokStitch?: boolean
  tiktokDisclose?: boolean
  tiktokYourBrand?: boolean
  tiktokBranded?: boolean
  instagramPostType?: "post" | "reel" | "story" | "carousel"
}
export type Delivery = {
  _id: string
  channel: Channel
  connectedAccountId: string
  status: "pending" | "publishing" | "published" | "failed" | "unknown"
  remotePostId?: string
  remoteUrl?: string
  error?: string
}
export type Post = {
  _id: string
  content: string
  targets: Target[]
  scheduledFor: number
  status: "draft" | "scheduled" | "publishing" | "published" | "failed"
  mediaIds: string[]
  options?: Options
  groupId?: string
  error?: string
  deliveries: Delivery[]
}
/** What a TikTok creator may post right now, from TikTok's creator info. */
export type Creator = {
  name: string
  privacy: string[]
  commentDisabled: boolean
  duetDisabled: boolean
  stitchDisabled: boolean
  maxDuration: number
}
export type Media = {
  _id: string
  storageId: string
  contentType: string
  name: string
  size: number
  url: string | null
}
export type Metric = { key: string; value: number; window: "period" | "snapshot" | "lifetime" }
export type Report = {
  channel: Channel
  connectedAccountId: string
  status: "ready" | "partial" | "target_required" | "permission_required" | "rate_limited" | "unsupported" | "error"
  metrics: Metric[]
  series: { key: string; points: { date: string; value: number }[] }[]
  note?: string
  permissionReason?: "approval_pending" | "reconnect_required"
  updatedAt: number
}

/** A failed Marketing request. `signIn` means the user must sign in to Cryzo first. */
export class MarketError extends Error {
  constructor(
    message: string,
    readonly signIn: boolean,
  ) {
    super(message)
  }
}

/** Calls the engine's `/api/market` proxy, which adds the Cryzo sign-in and forwards to Cryzo cloud. */
export function useMarketApi() {
  const sdk = useServerSDK()
  const platform = usePlatform()

  const request = async <T>(method: "GET" | "POST", path: string, body?: unknown): Promise<T> => {
    const password = sdk.http.password
    const response = await (platform.fetch ?? fetch)(`${sdk.url}/api/market/${path}`, {
      method,
      headers: {
        "Content-Type": "application/json",
        ...(password ? { Authorization: `Basic ${authTokenFromCredentials({ password })}` } : {}),
      },
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    })
    const payload = (await response.json().catch(() => undefined)) as
      | { data?: T; message?: string; kind?: string }
      | undefined
    if (!response.ok || !payload) throw new MarketError(payload?.message ?? "", payload?.kind === "cryzo_sign_in")
    return payload.data as T
  }

  return {
    providers: () => request<Provider[]>("GET", "providers"),
    accounts: () => request<{ accounts: Account[]; groups: Group[] }>("GET", "accounts"),
    /** The network's sign-in URL. The sign-in finishes in the browser; the account then appears in `accounts`. */
    connect: (channel: Channel) => request<string>("POST", "accounts/connect", { channel }),
    updateAccount: (input: { connectedAccountId: string; name?: string; groupId?: string | null }) =>
      request<null>("POST", "accounts/update", input),
    chooseTarget: (connectedAccountId: string, targetId: string) =>
      request<null>("POST", "accounts/target", { connectedAccountId, targetId }),
    disconnect: (connectedAccountId: string) => request<null>("POST", "accounts/disconnect", { connectedAccountId }),
    creator: (connectedAccountId: string) =>
      request<Creator | null>("GET", `accounts/creator?connectedAccountId=${encodeURIComponent(connectedAccountId)}`),
    targets: (connectedAccountId: string) =>
      request<{ id: string; name: string }[]>(
        "GET",
        `accounts/targets?connectedAccountId=${encodeURIComponent(connectedAccountId)}`,
      ),
    saveGroup: (input: { groupId?: string; name: string; color: string }) =>
      request<string>("POST", "groups/save", input),
    deleteGroup: (groupId: string) => request<null>("POST", "groups/delete", { groupId }),
    posts: (from: number, to: number) => request<Post[]>("GET", `posts?from=${from}&to=${to}`),
    createPost: (input: {
      requestKey: string
      content: string
      targets: Target[]
      scheduledFor: number
      status: "draft" | "scheduled"
      mediaIds: string[]
      options?: Options
      groupId?: string
    }) => request<string>("POST", "posts/create", input),
    updatePost: (
      input: Partial<Omit<Post, "_id" | "deliveries" | "error" | "groupId">> & {
        postId: string
        groupId?: string | null
      },
    ) => request<null>("POST", "posts/update", input),
    deletePost: (postId: string) => request<null>("POST", "posts/delete", { postId }),
    publish: (postId: string) => request<null>("POST", "posts/publish", { postId }),
    retry: (deliveryId: string) => request<null>("POST", "posts/retry", { deliveryId }),
    media: () => request<Media[]>("GET", "media"),
    upload: (path: string) => request<{ storageId: string }>("POST", "media/upload", { path }),
    deleteMedia: (storageId: string) => request<null>("POST", "media/delete", { storageId }),
    analytics: (days: number, refresh: boolean) =>
      request<Report[]>("GET", `analytics?days=${days}${refresh ? "&refresh=1" : ""}`),
    /** A `blob:` URL for stored media: the desktop's content policy blocks remote images. Revoke it when done. */
    file: async (url: string) => {
      const password = sdk.http.password
      const response = await (platform.fetch ?? fetch)(
        `${sdk.url}/api/market/media/file?url=${encodeURIComponent(url)}`,
        {
          headers: password ? { Authorization: `Basic ${authTokenFromCredentials({ password })}` } : {},
        },
      )
      if (!response.ok) throw new MarketError("", false)
      return URL.createObjectURL(await response.blob())
    },
  }
}

export type MarketApi = ReturnType<typeof useMarketApi>
