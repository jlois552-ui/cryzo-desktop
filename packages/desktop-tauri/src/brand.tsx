import { createUniqueId, type ComponentProps } from "solid-js"

// Same geometry, fading and outline contract as the upstream wordmark; only the lettering changes.
export function Wordmark(
  props: Pick<ComponentProps<"svg">, "class"> & { fade?: boolean; muted?: boolean; outline?: boolean },
) {
  const mask = createUniqueId()
  const gradient = createUniqueId()
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      viewBox="0 0 720 129"
      fill="none"
      classList={{
        [props.class ?? ""]: !!props.class,
        "overflow-visible [&_path]:[vector-effect:non-scaling-stroke]": props.outline,
      }}
    >
      <g opacity={props.muted === false ? 1 : 0.6} class="[[data-color-scheme=dark]_&]:opacity-100">
        <g mask={props.fade === false ? undefined : `url(#${mask})`}>
          <g
            transform="translate(129 0)"
            opacity={props.muted === false ? 1 : 0.112}
            fill={props.outline ? "none" : "currentColor"}
            stroke={props.outline ? "currentColor" : undefined}
            stroke-width={props.outline ? 1 : undefined}
          >
            <path pathLength={props.outline ? 1 : undefined} d="M74 18H0V110H74V92H18V36H74Z" />
            <path pathLength={props.outline ? 1 : undefined} d="M92 18H166V36H110V110H92Z" />
            <path pathLength={props.outline ? 1 : undefined} d="M184 18H202V55H240V18H258V73H230V110H212V73H184Z" />
            <path pathLength={props.outline ? 1 : undefined} d="M276 18H350V36L297 92H350V110H276V92L329 36H276Z" />
            <path
              pathLength={props.outline ? 1 : undefined}
              fill-rule="evenodd"
              d="M368 18H442V110H368ZM386 36V92H424V36Z"
            />
          </g>
        </g>
      </g>
      <defs>
        <mask id={mask} style="mask-type:alpha" maskUnits="userSpaceOnUse" x="0" y="0" width="720" height="129">
          <rect width="720" height="129" fill={`url(#${gradient})`} />
        </mask>
        <linearGradient id={gradient} x1="360" y1="68" x2="360" y2="129" gradientUnits="userSpaceOnUse">
          <stop stop-color="white" stop-opacity="0.7" />
          <stop offset="1" stop-color="white" stop-opacity="0" />
        </linearGradient>
      </defs>
    </svg>
  )
}

// Branding only: the upstream components continue to own layout and sizing.
export function Mark(props: { class?: string }) {
  return (
    <svg
      data-component="logo-mark"
      class={props.class}
      viewBox="0 0 16 20"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
    >
      <path d="M14 5H4V15H14M9 5V2H1V18H9V15" stroke="var(--icon-strong-base)" stroke-width="2" />
    </svg>
  )
}

export function Splash(props: Pick<ComponentProps<"svg">, "ref" | "class">) {
  return (
    <svg
      ref={props.ref}
      data-component="logo-splash"
      class={props.class}
      viewBox="0 0 80 100"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
    >
      <path d="M70 25H20V75H70M45 25V10H5V90H45V75" stroke="var(--icon-strong-base)" stroke-width="10" />
    </svg>
  )
}

export function Logo(props: { class?: string }) {
  return (
    <svg class={props.class} viewBox="0 0 234 42" fill="var(--icon-strong-base)" xmlns="http://www.w3.org/2000/svg">
      <text x="0" y="34" font-size="40" font-weight="600" style={{ "font-family": "var(--font-family-sans)" }}>
        Cryzo
      </text>
    </svg>
  )
}
