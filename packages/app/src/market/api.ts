import { authTokenFromCredentials } from "@/runtime/server/api"
import { useServerSDK } from "@/runtime/server/client"
import { usePlatform } from "@/runtime/platform/platform"

export const channels = ["x", "facebook", "instagram", "youtube", "tiktok", "reddit"] as const
export type Channel = (typeof channels)[number]

export type Group = { _id: string; name: string; color: string }
export type Account = {
  _id: string
  channel: Channel
  connectedAccountId: string
  name: string
  avatar?: string
  targetId?: string
  groupId?: string
}
export type Target = { channel: Channel; connectedAccountId: string; content?: string }
export type Options = {
  redditCommunity?: string
  youtubeTitle?: string
  youtubePrivacy?: string
  tiktokPrivacy?: string
  instagramPostType?: "post" | "reel" | "story" | "carousel"
}
export type Delivery = {
  _id: string
  channel: Channel
  connectedAccountId: string
  status: "pending" | "publishing" | "published" | "failed" | "unknown"
  remotePostId?: string
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
export type Media = { _id: string; storageId: string; contentType: string; name: string; size: number; url: string | null }
export type Metric = { key: string; value: number; window: "period" | "snapshot" | "lifetime" }
export type Report = {
  channel: Channel
  connectedAccountId: string
  status: "ready" | "partial" | "target_required" | "permission_required" | "rate_limited" | "unsupported" | "error"
  metrics: Metric[]
  series: { key: string; points: { date: string; value: number }[] }[]
  note?: string
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
    accounts: () => request<{ accounts: Account[]; groups: Group[] }>("GET", "accounts"),
    sync: () => request<{ accounts: Account[]; groups: Group[] }>("POST", "accounts/sync"),
    connect: (channel: Channel) => request<string>("POST", "accounts/connect", { channel }),
    updateAccount: (input: { connectedAccountId: string; name?: string; targetId?: string; groupId?: string | null }) =>
      request<null>("POST", "accounts/update", input),
    disconnect: (connectedAccountId: string) => request<null>("POST", "accounts/disconnect", { connectedAccountId }),
    targets: (connectedAccountId: string) =>
      request<{ id: string; name: string }[]>("GET", `accounts/targets?connectedAccountId=${encodeURIComponent(connectedAccountId)}`),
    saveGroup: (input: { groupId?: string; name: string; color: string }) => request<string>("POST", "groups/save", input),
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
    updatePost: (input: Partial<Omit<Post, "_id" | "deliveries" | "error" | "groupId">> & { postId: string; groupId?: string | null }) =>
      request<null>("POST", "posts/update", input),
    deletePost: (postId: string) => request<null>("POST", "posts/delete", { postId }),
    publish: (postId: string) => request<null>("POST", "posts/publish", { postId }),
    retry: (deliveryId: string) => request<null>("POST", "posts/retry", { deliveryId }),
    media: () => request<Media[]>("GET", "media"),
    upload: (path: string) => request<{ storageId: string }>("POST", "media/upload", { path }),
    deleteMedia: (storageId: string) => request<null>("POST", "media/delete", { storageId }),
    analytics: (days: number, refresh: boolean) => request<Report[]>("GET", `analytics?days=${days}${refresh ? "&refresh=1" : ""}`),
    /** A `blob:` URL for stored media: the desktop's content policy blocks remote images. Revoke it when done. */
    file: async (url: string) => {
      const password = sdk.http.password
      const response = await (platform.fetch ?? fetch)(`${sdk.url}/api/market/media/file?url=${encodeURIComponent(url)}`, {
        headers: password ? { Authorization: `Basic ${authTokenFromCredentials({ password })}` } : {},
      })
      if (!response.ok) throw new MarketError("", false)
      return URL.createObjectURL(await response.blob())
    },
  }
}

export type MarketApi = ReturnType<typeof useMarketApi>
