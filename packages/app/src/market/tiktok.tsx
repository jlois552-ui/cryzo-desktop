import { Loader } from "@opencode/ui/loader"
import { For, Show, createEffect, createMemo, onCleanup } from "solid-js"
import { createStore } from "solid-js/store"
import { useLanguage } from "@/runtime/i18n/language"
import { usePlatform } from "@/runtime/platform/platform"
import type { Creator, Media, Options, Target } from "./api"
import type { MarketModel } from "./model"

const privacyNames = {
  SELF_ONLY: "self",
  MUTUAL_FOLLOW_FRIENDS: "friends",
  FOLLOWER_OF_CREATOR: "followers",
  PUBLIC_TO_EVERYONE: "public",
} as const
const interactions = [
  { name: "comment", option: "tiktokComment", disabled: "commentDisabled" },
  { name: "duet", option: "tiktokDuet", disabled: "duetDisabled" },
  { name: "stitch", option: "tiktokStitch", disabled: "stitchDisabled" },
] as const
const brands = [
  { name: "yourBrand", option: "tiktokYourBrand" },
  { name: "branded", option: "tiktokBranded" },
] as const
const musicPolicy = "https://www.tiktok.com/legal/page/global/music-usage-confirmation/en"
const brandedPolicy = "https://www.tiktok.com/legal/page/global/bc-policy/en"

/**
 * TikTok's Direct Post settings. TikTok's content sharing guidelines require showing the creator, a visibility the
 * creator picks with nothing preselected, interactions that start off and follow the account's settings, commercial
 * content disclosure, the duration limit and consent wording. A draft skips them: the creator sets them in TikTok.
 */
export function TikTokSettings(props: {
  model: MarketModel
  targets: Target[]
  video?: Media
  options: Options
  locked: boolean
  onOption: <K extends keyof Options>(key: K, value: Options[K]) => void
  onBlocked: (blocked: boolean) => void
}) {
  const language = useLanguage()
  const platform = usePlatform()
  const [state, setState] = createStore({
    creators: {} as Record<string, Creator | "loading" | "error">,
    src: undefined as string | undefined,
    duration: 0,
  })
  createEffect(() => {
    for (const target of props.targets) {
      const id = target.connectedAccountId
      if (state.creators[id]) continue
      setState("creators", id, "loading")
      props.model.api.creator(id).then(
        (creator) => setState("creators", id, creator ?? "error"),
        () => setState("creators", id, "error"),
      )
    }
  })
  // The preview doubles as the duration check, since TikTok limits length per creator.
  createEffect(() => {
    const url = props.video?.url
    if (!url) return
    let current: string | undefined
    let live = true
    props.model.api
      .file(url)
      .then((src) => {
        if (live) setState("src", (current = src))
        else URL.revokeObjectURL(src)
      })
      .catch(() => undefined)
    onCleanup(() => {
      live = false
      if (current) URL.revokeObjectURL(current)
      setState({ src: undefined, duration: 0 })
    })
  })

  const status = (target: Target) => state.creators[target.connectedAccountId]
  const creators = createMemo(() =>
    props.targets.map(status).filter((creator): creator is Creator => typeof creator === "object"),
  )
  const failed = () => props.targets.some((target) => status(target) === "error")
  const loading = () => props.targets.some((target) => !status(target) || status(target) === "loading")
  const privacy = createMemo(() =>
    (creators()[0]?.privacy ?? []).filter((level) => creators().every((creator) => creator.privacy.includes(level))),
  )
  const off = (key: (typeof interactions)[number]["disabled"]) => creators().some((creator) => creator[key])
  const maxDuration = () => Math.min(...creators().map((creator) => creator.maxDuration))
  const tooLong = () => creators().length > 0 && state.duration > maxDuration()
  const draft = () => props.options.tiktokMode === "draft"
  const branded = () => !!props.options.tiktokDisclose && !!props.options.tiktokBranded
  const label = () => {
    if (branded()) return "market.editor.tiktok.labelPaid" as const
    if (props.options.tiktokYourBrand) return "market.editor.tiktok.labelPromo" as const
    return "market.editor.tiktok.chooseDisclosure" as const
  }

  createEffect(() => props.onBlocked(!draft() && (loading() || failed() || tooLong())))
  onCleanup(() => props.onBlocked(false))
  // Clear a visibility the account no longer offers, or "Only me" once branded content is disclosed.
  createEffect(() => {
    const level = props.options.tiktokPrivacy
    if (props.locked || draft() || !level || !creators().length) return
    if (!privacy().includes(level) || (branded() && level === "SELF_ONLY")) props.onOption("tiktokPrivacy", undefined)
  })

  const field =
    "h-9 w-full rounded-[8px] border border-v2-border-border-base bg-v2-background-bg-base px-3 text-[13px] text-v2-text-text-base"
  const hint = "text-[12px] leading-4 text-v2-text-text-muted"
  const link = "text-[12px] leading-4 text-v2-text-text-base underline"

  return (
    <section class="flex flex-col gap-3 rounded-[12px] border border-v2-border-border-muted p-3">
      <div class="flex flex-wrap items-center gap-2">
        <span class="text-[13px] font-medium leading-5 text-v2-text-text-base">
          {language.t("market.editor.tiktok.title")}
        </span>
        <div class="flex-1" />
        <Show when={loading() && !failed()}>
          <Loader class="size-4" />
        </Show>
        <For each={creators()}>
          {(creator) => (
            <span class={hint}>{language.t("market.editor.tiktok.postingAs", { name: creator.name })}</span>
          )}
        </For>
      </div>
      <Show when={failed() && !draft()}>
        <p class="text-[12px] leading-4 text-v2-state-fg-danger">{language.t("market.editor.tiktok.unavailable")}</p>
      </Show>
      <div class="flex gap-4">
        <Show when={state.src}>
          {(src) => (
            <video
              src={src()}
              controls
              muted
              class="aspect-[9/16] w-[120px] shrink-0 rounded-[8px] bg-v2-background-bg-layer-03 object-contain"
              onLoadedMetadata={(event) => setState("duration", event.currentTarget.duration)}
            />
          )}
        </Show>
        <div class="flex min-w-0 flex-1 flex-col gap-3">
          <label class="flex flex-col gap-1.5 text-[12px] leading-4 text-v2-text-text-muted">
            {language.t("market.editor.tiktok.mode")}
            <select
              class={field}
              value={props.options.tiktokMode ?? "direct"}
              disabled={props.locked}
              onChange={(event) =>
                props.onOption("tiktokMode", event.currentTarget.value === "draft" ? "draft" : "direct")
              }
            >
              <option value="direct">{language.t("market.editor.tiktok.modeDirect")}</option>
              <option value="draft">{language.t("market.editor.tiktok.modeDraft")}</option>
            </select>
          </label>
          <Show
            when={!draft()}
            fallback={<span class={hint}>{language.t("market.editor.tiktok.draftHint")}</span>}
          >
            <label class="flex flex-col gap-1.5 text-[12px] leading-4 text-v2-text-text-muted">
              {language.t("market.editor.tiktokPrivacy")}
              <select
                class={field}
                value={props.options.tiktokPrivacy ?? ""}
                disabled={props.locked || !creators().length}
                onChange={(event) => props.onOption("tiktokPrivacy", event.currentTarget.value || undefined)}
              >
                <option value="" disabled>
                  {language.t("market.editor.tiktok.choosePrivacy")}
                </option>
                <For each={privacy()}>
                  {(level) => {
                    const name = privacyNames[level as keyof typeof privacyNames]
                    return (
                      <option value={level} disabled={level === "SELF_ONLY" && branded()}>
                        {name ? language.t(`market.editor.tiktok.${name}`) : level}
                      </option>
                    )
                  }}
                </For>
              </select>
              <Show when={branded()}>
                <span>{language.t("market.editor.tiktok.brandedPrivate")}</span>
              </Show>
            </label>

            <fieldset class="flex flex-col gap-1.5">
              <legend class={`pb-1.5 ${hint}`}>{language.t("market.editor.tiktok.allow")}</legend>
              <div class="flex flex-wrap gap-4">
                <For each={interactions}>
                  {(item) => (
                    <label class="flex items-center gap-2 text-[13px] leading-5 text-v2-text-text-base">
                      <input
                        type="checkbox"
                        checked={!!props.options[item.option] && !off(item.disabled)}
                        disabled={props.locked || !creators().length || off(item.disabled)}
                        onChange={(event) => props.onOption(item.option, event.currentTarget.checked)}
                      />
                      {language.t(`market.editor.tiktok.${item.name}`)}
                    </label>
                  )}
                </For>
              </div>
              <Show when={interactions.some((item) => off(item.disabled))}>
                <span class={hint}>{language.t("market.editor.tiktok.interactionOff")}</span>
              </Show>
            </fieldset>

            <div class="flex flex-col gap-2 rounded-[8px] bg-v2-background-bg-layer-01 p-3">
              <label class="flex items-start gap-2">
                <input
                  type="checkbox"
                  class="mt-1"
                  checked={!!props.options.tiktokDisclose}
                  disabled={props.locked}
                  onChange={(event) => props.onOption("tiktokDisclose", event.currentTarget.checked)}
                />
                <span class="flex flex-col">
                  <span class="text-[13px] leading-5 text-v2-text-text-base">
                    {language.t("market.editor.tiktok.disclose")}
                  </span>
                  <span class={hint}>{language.t("market.editor.tiktok.discloseHint")}</span>
                </span>
              </label>
              <Show when={props.options.tiktokDisclose}>
                <For each={brands}>
                  {(item) => (
                    <label class="flex items-start gap-2 ps-6">
                      <input
                        type="checkbox"
                        class="mt-1"
                        checked={!!props.options[item.option]}
                        disabled={props.locked}
                        onChange={(event) => props.onOption(item.option, event.currentTarget.checked)}
                      />
                      <span class="flex flex-col">
                        <span class="text-[13px] leading-5 text-v2-text-text-base">
                          {language.t(`market.editor.tiktok.${item.name}`)}
                        </span>
                        <span class={hint}>{language.t(`market.editor.tiktok.${item.name}Hint`)}</span>
                      </span>
                    </label>
                  )}
                </For>
                <span
                  class="ps-6 text-[12px] leading-4"
                  classList={{
                    "text-v2-state-fg-warning": label() === "market.editor.tiktok.chooseDisclosure",
                    "text-v2-text-text-muted": label() !== "market.editor.tiktok.chooseDisclosure",
                  }}
                >
                  {language.t(label())}
                </span>
              </Show>
            </div>

            <Show when={tooLong()}>
              <p class="text-[12px] leading-4 text-v2-state-fg-danger">
                {language.t("market.editor.tiktok.tooLong", {
                  seconds: Math.ceil(state.duration),
                  max: maxDuration(),
                })}
              </p>
            </Show>
            <div class="flex flex-col gap-1">
              <span class={hint}>
                {language.t(branded() ? "market.editor.tiktok.consentBranded" : "market.editor.tiktok.consent")}
              </span>
              <div class="flex flex-wrap gap-3">
                <Show when={branded()}>
                  <button type="button" class={link} onClick={() => platform.openExternal(brandedPolicy)}>
                    {language.t("market.editor.tiktok.brandedPolicy")}
                  </button>
                </Show>
                <button type="button" class={link} onClick={() => platform.openExternal(musicPolicy)}>
                  {language.t("market.editor.tiktok.musicPolicy")}
                </button>
              </div>
              <span class={hint}>{language.t("market.editor.tiktok.processing")}</span>
            </div>
          </Show>
        </div>
      </div>
    </section>
  )
}
