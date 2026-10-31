import { Button } from "@opencode/ui/button"
import { useDialog } from "@opencode/ui/context/dialog"
import { Loader } from "@opencode/ui/loader"
import { useSearchParams } from "@solidjs/router"
import { For, Match, Show, Switch, createEffect, createMemo, onMount, type JSX } from "solid-js"
import { createStore } from "solid-js/store"
import { useLanguage } from "@/runtime/i18n/language"
import { ServerProvider } from "@/runtime/server/current"
import { ServerConnection } from "@/runtime/server/registry"
import { useGlobal } from "@/runtime/server/runtime"
import { useTabs } from "@/shell/tabs/tabs"
import type { Post } from "./api"
import { Analytics } from "./analytics"
import { Calendar } from "./calendar"
import { ChannelsPage } from "./channels"
import { PostEditor } from "./editor"
import { MarketIcon } from "./icons"
import { MediaLibrary } from "./media"
import { createMarketModel, sections, type Section } from "./model"
import { Overview } from "./overview"
import { Posts } from "./posts"

/** Marketing runs against the local Cryzo engine, which forwards to Cryzo cloud with the user's sign-in. */
export function MarketRoute() {
  const global = useGlobal()
  const conn = createMemo(() => {
    const servers = global.servers.list()
    return servers.find((item) => item.type === "sidecar") ?? servers[0]
  })
  return (
    <Show when={conn()} keyed>
      {(conn) => (
        <ServerProvider conn={conn}>
          <MarketScreen server={conn} />
        </ServerProvider>
      )}
    </Show>
  )
}

function MarketScreen(props: { server: ServerConnection.Any }) {
  const language = useLanguage()
  const dialog = useDialog()
  const global = useGlobal()
  const tabs = useTabs()
  const model = createMarketModel()

  /** Opens a new Cryzo session in the most recent project with a marketing brief, so the agent's
   * marketing tools draft and schedule posts that then appear on this calendar. */
  const ask = (text: string) => {
    const directory = global.ensureServerCtx(props.server).projects.used()[0]?.worktree
    if (!directory) return
    const brief = language.t("market.chat.instructions", { now: localTime() })
    void tabs.newDraft({ server: ServerConnection.key(props.server), directory }, `${brief}

${text}`.trim())
  }
  // Wix's "Marketing ideas drafted for you": the agent writes drafts that wait in Overview for approval.
  const plan = () => ask(language.t("market.chat.plan"))

  // `?section=` selects the page, so the app sidebar can link straight to one.
  const [params, setParams] = useSearchParams<{ section?: string }>()
  createEffect(() => {
    const section = sections.find((item) => item === params.section)
    if (section && section !== model.state.section) model.setState("section", section)
  })
  const go = (section: Section) => {
    model.setState("section", section)
    setParams({ section })
  }
  onMount(() => void model.load())

  const open = (post: Post) => dialog.show(() => <PostEditor model={model} post={post} />)
  const create = (at?: number) => dialog.show(() => <PostEditor model={model} at={at} />)
  const icons: Record<Section, "overview" | "posts" | "calendar" | "analytics" | "media" | "channels"> = {
    overview: "overview",
    posts: "posts",
    calendar: "calendar",
    analytics: "analytics",
    media: "media",
    channels: "channels",
  }

  return (
    <div class="flex min-h-0 flex-1 px-[var(--shell-inline-inset,8px)] pb-[var(--shell-bottom-inset,8px)] pt-[var(--shell-top-inset,8px)]">
      <div class="flex min-h-0 flex-1 overflow-hidden rounded-[12px] bg-v2-background-bg-base">
        {/* On desktop the app sidebar lists these pages; phones have no sidebar. */}
        <nav class="flex w-[200px] shrink-0 flex-col gap-0.5 border-e border-v2-border-border-muted px-2 py-4 md:hidden" aria-label={language.t("market.title")}>
          <div class="mb-2 px-2 text-[12px] font-medium leading-4 text-v2-text-text-faint">{language.t("market.title")}</div>
          <For each={sections}>
            {(section) => (
              <button
                type="button"
                class="flex h-8 items-center gap-2.5 rounded-[8px] px-2 text-[13px] leading-5 text-v2-text-text-muted hover:bg-v2-overlay-simple-overlay-hover aria-[current=page]:bg-v2-background-bg-layer-03 aria-[current=page]:text-v2-text-text-base"
                aria-current={model.state.section === section ? "page" : undefined}
                onClick={() => go(section)}
              >
                <MarketIcon name={icons[section]} class="size-4" />
                {language.t(`market.nav.${section}`)}
              </button>
            )}
          </For>
        </nav>
        <Switch>
          <Match when={!model.state.ready}>
            <div class="grid flex-1 place-items-center">
              <Loader class="size-5" />
            </div>
          </Match>
          <Match when={model.state.signIn}>
            <SignIn onRetry={() => void model.load()} />
          </Match>
          <Match when={true}>
            <div class="flex min-h-0 min-w-0 flex-1 flex-col">
              <header class="flex flex-wrap items-center gap-3 px-6 pb-3 pt-5">
                <div class="min-w-0 flex-1">
                  <h1 class="text-[20px] font-medium leading-7 text-v2-text-text-base">{language.t(`market.nav.${model.state.section}`)}</h1>
                  <p class="text-[13px] leading-5 text-v2-text-text-muted">{language.t(`market.page.${model.state.section}`)}</p>
                </div>
                <Button variant="neutral" size="large" disabled={!model.state.accounts.length} onClick={plan}>
                  <MarketIcon name="sparkle" class="size-4" />
                  {language.t("market.overview.ideas.action")}
                </Button>
                <Button variant="contrast" size="large" icon="plus" disabled={!model.state.accounts.length} onClick={() => create()}>
                  {language.t("market.channels.create")}
                </Button>
              </header>
              <div class="flex min-h-0 flex-1 flex-col overflow-y-auto px-6 pb-5" data-scrollable>
                <Switch>
                  <Match when={model.state.section === "overview"}>
                    <Overview model={model} onPlan={plan} onOpen={open} onSection={go} />
                  </Match>
                  <Match when={model.state.section === "posts"}>
                    <Posts model={model} onPlan={plan} onOpen={open} onCreate={(at) => create(at)} />
                  </Match>
                  <Match when={model.state.section === "calendar"}>
                    <Calendar model={model} onOpen={open} onCreate={(at) => create(at)} />
                  </Match>
                  <Match when={model.state.section === "analytics"}>
                    <Analytics model={model} />
                  </Match>
                  <Match when={model.state.section === "media"}>
                    <MediaLibrary model={model} />
                  </Match>
                  <Match when={model.state.section === "channels"}>
                    <ChannelsPage model={model} />
                  </Match>
                </Switch>
              </div>
              <ChatBar onSend={ask} />
            </div>
          </Match>
        </Switch>
      </div>
    </div>
  )
}

/** Local time as ISO 8601 with its UTC offset, e.g. 2026-11-02T09:00:00-05:00, the format the marketing tools accept. */
function localTime() {
  const now = new Date()
  const offset = -now.getTimezoneOffset()
  const pad = (value: number) => String(Math.floor(Math.abs(value))).padStart(2, "0")
  const local = new Date(now.getTime() + offset * 60_000).toISOString().slice(0, 19)
  return `${local}${offset < 0 ? "-" : "+"}${pad(offset / 60)}:${pad(offset % 60)}`
}

function ChatBar(props: { onSend: (text: string) => void }) {
  const language = useLanguage()
  const [state, setState] = createStore({ text: "" })
  const send = () => {
    const text = state.text.trim()
    if (!text) return
    setState("text", "")
    props.onSend(text)
  }
  return (
    <form
      class="flex items-center gap-2 border-t border-v2-border-border-muted px-5 py-3"
      onSubmit={(event) => {
        event.preventDefault()
        send()
      }}
    >
      <MarketIcon name="sparkle" class="size-5 shrink-0 text-v2-icon-icon-accent" />
      <input
        class="h-10 min-w-0 flex-1 rounded-[10px] border border-v2-border-border-base bg-v2-background-bg-base px-3 text-[14px] text-v2-text-text-base"
        placeholder={language.t("market.chat.placeholder")}
        value={state.text}
        onInput={(event) => setState("text", event.currentTarget.value)}
      />
      <Button type="submit" variant="contrast" disabled={!state.text.trim()}>
        {language.t("market.chat.send")}
      </Button>
    </form>
  )
}

function SignIn(props: { onRetry: () => void }): JSX.Element {
  const language = useLanguage()
  const dialog = useDialog()
  const connect = async () => {
    const { DialogConnectProvider, useProviderConnectController } = await import("@/providers/connect/dialog")
    const controller = useProviderConnectController()
    controller.select("cryzo")
    void dialog.show(() => <DialogConnectProvider controller={controller} standalone onConnected={() => props.onRetry()} />)
  }
  return (
    <div class="grid flex-1 place-items-center p-8">
      <div class="flex max-w-[380px] flex-col items-center gap-4 text-center">
        <MarketIcon name="calendar" class="size-10 text-v2-icon-icon-muted" />
        <h2 class="text-[20px] font-medium leading-7 text-v2-text-text-base">{language.t("market.signIn.title")}</h2>
        <p class="text-[13px] leading-5 text-v2-text-text-muted">{language.t("market.signIn.description")}</p>
        <div class="flex gap-2">
          <Button variant="contrast" size="large" onClick={() => void connect()}>
            {language.t("market.signIn.action")}
          </Button>
          <Button variant="ghost" size="large" onClick={props.onRetry}>
            {language.t("common.retry")}
          </Button>
        </div>
      </div>
    </div>
  )
}
