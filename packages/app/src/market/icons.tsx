import type { JSX } from "solid-js"
import type { Account, Channel } from "./api"

// Rail glyphs drawn in currentColor so they follow the theme; the shared catalog has no calendar or chart.
const paths: Record<
  "overview" | "posts" | "calendar" | "analytics" | "media" | "channels" | "sparkle" | "sidebar" | "compose",
  JSX.Element
> = {
  sidebar: (
    <>
      <rect x="3.5" y="4.5" width="17" height="15" rx="2.5" />
      <path d="M9.5 4.5v15" />
    </>
  ),
  compose: <path d="M11 4.5H6.5a2 2 0 0 0-2 2v11a2 2 0 0 0 2 2h11a2 2 0 0 0 2-2V13M17.6 3.9a1.9 1.9 0 0 1 2.7 2.7L12.5 14.4l-3.5.9.9-3.5z" />,
  overview: <path d="M4 10.5 12 4l8 6.5V19a1 1 0 0 1-1 1h-4.5v-5.5h-5V20H5a1 1 0 0 1-1-1z" />,
  posts: (
    <>
      <rect x="4" y="4" width="16" height="16" rx="2.5" />
      <path d="M8 9h8M8 12.5h8M8 16h5" />
    </>
  ),
  channels: <path d="M10 14a4 4 0 0 0 5.66 0l3-3a4 4 0 0 0-5.66-5.66l-1 1M14 10a4 4 0 0 0-5.66 0l-3 3a4 4 0 0 0 5.66 5.66l1-1" />,
  calendar: (
    <>
      <rect x="3.5" y="5" width="17" height="15" rx="2.5" />
      <path d="M3.5 10h17M8 3v4M16 3v4" />
    </>
  ),
  analytics: <path d="M4 20V4M4 20h16M8 16l4-5 3 3 5-6" />,
  media: (
    <>
      <rect x="3.5" y="4.5" width="17" height="15" rx="2.5" />
      <path d="M3.5 16l5-5 4 4 3-3 5 5" />
      <circle cx="15.5" cy="9" r="1.5" />
    </>
  ),
  sparkle: <path d="M12 3l1.8 5.2L19 10l-5.2 1.8L12 17l-1.8-5.2L5 10l5.2-1.8zM18.5 15.5l.8 2.2 2.2.8-2.2.8-.8 2.2-.8-2.2-2.2-.8 2.2-.8z" />,
}

export function MarketIcon(props: { name: keyof typeof paths; class?: string }) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="1.6"
      stroke-linecap="round"
      stroke-linejoin="round"
      class={props.class ?? "size-5"}
      aria-hidden="true"
    >
      {paths[props.name]}
    </svg>
  )
}

const brand: Record<Channel, { color: string; mark: string }> = {
  x: { color: "#000000", mark: "X" },
  linkedin: { color: "#0A66C2", mark: "in" },
  facebook: { color: "#1877F2", mark: "f" },
  instagram: { color: "#E1306C", mark: "IG" },
  youtube: { color: "#FF0000", mark: "▶" },
  tiktok: { color: "#010101", mark: "♪" },
  reddit: { color: "#FF4500", mark: "r" },
}

export function NetworkBadge(props: { channel: Channel; class?: string }) {
  return (
    <span
      class={`grid shrink-0 place-items-center rounded-full font-semibold text-white ring-2 ring-v2-background-bg-base ${props.class ?? "size-4 text-[8px] leading-none"}`}
      style={{ "background-color": brand[props.channel].color, color: "#ffffff" }}
      aria-hidden="true"
    >
      {brand[props.channel].mark}
    </span>
  )
}

/** Initial with the network badge in the corner, as in Postiz's channel list. Profile pictures live on
 * each network's CDN, which the desktop's content policy does not load. */
export function ChannelAvatar(props: { account: Pick<Account, "channel" | "name">; size?: "small" | "large" }) {
  const large = () => props.size !== "small"
  return (
    <span class={`relative shrink-0 ${large() ? "size-9" : "size-6"}`}>
      <span
        class={`grid size-full place-items-center rounded-full bg-v2-background-bg-layer-03 font-medium text-v2-text-text-base ${large() ? "text-[13px]" : "text-[10px]"}`}
      >
        {props.account.name.slice(0, 1).toUpperCase()}
      </span>
      <span class="absolute -bottom-0.5 -end-0.5">
        <NetworkBadge channel={props.account.channel} class={large() ? "size-4 text-[8px] leading-none" : "size-3 text-[6px] leading-none"} />
      </span>
    </span>
  )
}
