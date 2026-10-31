// Direct OAuth and publishing for each supported network. Each provider covers one network's
// sign-in, token refresh, profile, page or account choice, publishing and analytics.
// The flow follows Postiz's integrations (gitroomhq/postiz-app); none of its code is used.
import type { Channel } from "../market"
import { facebook, instagram } from "./meta"
import { linkedin } from "./linkedin"
import { reddit } from "./reddit"
import { tiktok } from "./tiktok"
import { x } from "./x"
import type { Client, Provider } from "./net"
import { youtube } from "./youtube"

export * from "./net"

export const providers: Record<Channel, Provider> = { x, linkedin, facebook, instagram, youtube, tiktok, reddit }

export function client(channel: Channel): Client | null {
  const [id, secret] = providers[channel].env.map((name) => process.env[name]?.trim())
  return id && secret ? { id, secret } : null
}

export function redirect(channel: Channel) {
  return `${process.env.SITE_URL}/market/oauth/${channel}/callback`
}
