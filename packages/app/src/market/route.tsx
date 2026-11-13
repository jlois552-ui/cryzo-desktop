import { Dialog, DialogTitle } from "@opencode/ui/dialog"
import { ChannelSidebar } from "./channel-sidebar"
import "./market.css"
import { Button } from "@opencode/ui/button"
import { useDialog } from "@opencode/ui/context/dialog"
import { Loader } from "@opencode/ui/loader"
import { useSearchParams } from "@solidjs/router"
import {
  Match,
  Show,
  Switch,
  Suspense,
  lazy,
  createEffect,
  createMemo,
  onMount,
  onCleanup,
  type JSX,
} from "solid-js"
import { createStore } from "solid-js/store"
import { useLanguage } from "@/runtime/i18n/language"
import { ServerProvider } from "@/runtime/server/current"
import { ServerConnection } from "@/runtime/server/registry"
import { useGlobal } from "@/runtime/server/runtime"
import type { Post } from "./api"
import { Analytics } from "./analytics"
import { Calendar } from "./calendar"
import { ChannelsPage } from "./channels"
import { PostEditor } from "./editor"
import { MarketIcon } from "./icons"
import { MediaLibrary } from "./media"
import { createMarketModel, sections } from "./model"

const MarketingAgent = lazy(() => import("./agent").then((module) => ({ default: module.MarketingAgent })))

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
  const model = createMarketModel()
  const [state, setState] = createStore({ agent: false, width: 1200 })
  let root!: HTMLDivElement
  const directory = () => global.ensureServerCtx(props.server).projects.used()[0]?.worktree
  const [params, setParams] = useSearchParams<{ section?: string }>()
  const manage = () =>
    dialog.show(() => (
      <Dialog fit containerClass="!w-[min(calc(100vw_-_40px),900px)] !max-h-[85vh]">
        <div class="flex min-h-0 flex-col gap-4 p-5">
          <DialogTitle>{language.t("market.nav.channels")}</DialogTitle>
          <div class="min-h-0 overflow-y-auto" data-scrollable>
            <ChannelsPage model={model} />
          </div>
        </div>
      </Dialog>
    ))
  createEffect(() => {
    const section = params.section
    if (section === "channels") {
      setParams({ section: "calendar" }, { replace: true })
      manage()
      return
    }
    if (section === "posts") {
      model.setView("list")
      setParams({ section: "calendar" }, { replace: true })
      return
    }
    const valid = sections.find((item) => item === section)
    if (valid) model.setState("section", valid)
    else {
      model.setState("section", "calendar")
      if (section) setParams({ section: "calendar" }, { replace: true })
    }
  })
  const refresh = () => void model.load()
  onMount(() => {
    refresh()
    const observer = new ResizeObserver(([entry]) => setState("width", entry.contentRect.width))
    observer.observe(root)
    window.addEventListener("focus", refresh)
    onCleanup(() => {
      observer.disconnect()
      window.removeEventListener("focus", refresh)
    })
  })
  const open = (post: Post) => dialog.show(() => <PostEditor model={model} post={post} />)
  const create = (at?: number) =>
    dialog.show(() => (
      <PostEditor
        model={model}
        at={at}
        targets={
          model.state.selectedAccounts === null
            ? undefined
            : model.state.accounts
                .filter(
                  (account) =>
                    model.state.selectedAccounts!.includes(account.connectedAccountId) &&
                    !account.needsReconnect &&
                    !account.needsTarget,
                )
                .map((account) => ({ channel: account.channel, connectedAccountId: account.connectedAccountId }))
        }
      />
    ))
  const context = () => `${language.t("market.chat.instructions", { now: localTime() })}
Marketing context: ${JSON.stringify({ section: model.state.section, view: model.state.view, anchor: new Date(model.state.anchor).toISOString(), timezone: Intl.DateTimeFormat().resolvedOptions().timeZone, selectedAccounts: model.state.selectedAccounts, status: model.state.status, group: model.state.group })}`
  return (
    <div class="flex min-h-0 flex-1 px-[var(--shell-inline-inset,8px)] pb-[var(--shell-bottom-inset,8px)] pt-[var(--shell-top-inset,8px)]">
      <div
        ref={root}
        class="market-workspace relative flex min-h-0 min-w-0 flex-1 overflow-hidden rounded-xl bg-v2-background-bg-base"
        onKeyDown={(event) => {
          if (event.key === "Escape" && state.agent) setState("agent", false)
        }}
      >
        <Switch>
          <Match when={!model.state.ready}>
            <div class="grid flex-1 place-items-center">
              <Loader class="size-5" />
            </div>
          </Match>
          <Match when={model.state.signIn}>
            <SignIn onRetry={refresh} />
          </Match>
          <Match when={true}>
            <div class="flex min-h-0 min-w-0 flex-1 flex-col">
              {/* Calendar, Analytics and Media are picked in the app sidebar, so the page shows only where you are. */}
              <header class="flex h-12 shrink-0 items-center gap-2 border-b border-v2-border-border-muted px-4">
                <h1 class="min-w-0 flex-1 truncate text-[14px] font-medium text-v2-text-text-base">
                  {language.t(`market.nav.${model.state.section}`)}
                </h1>
                <Button variant="neutral" onClick={() => setState("agent", !state.agent)} aria-expanded={state.agent}>
                  <MarketIcon name="sparkle" class="size-4" />
                  {language.t("market.agent.title")}
                </Button>
              </header>
              <div class="flex min-h-0 min-w-0 flex-1">
                <Show when={model.state.section === "calendar"}>
                  <ChannelSidebar
                    model={model}
                    compact={model.state.collapsed || state.width < (state.agent ? 1240 : 760)}
                    onManage={manage}
                    onCreate={() => create()}
                  />
                </Show>
                <div class="flex min-h-0 min-w-0 flex-1 flex-col overflow-y-auto p-4" data-scrollable>
                  <Switch>
                    <Match when={model.state.section === "calendar"}>
                      <Calendar model={model} onOpen={open} onCreate={create} />
                    </Match>
                    <Match when={model.state.section === "analytics"}>
                      <Analytics model={model} />
                    </Match>
                    <Match when={model.state.section === "media"}>
                      <MediaLibrary model={model} />
                    </Match>
                  </Switch>
                </div>
              </div>
            </div>
            <Show when={state.agent}>
              <Suspense
                fallback={
                  <div class="grid w-[400px] place-items-center">
                    <Loader class="size-4" />
                  </div>
                }
              >
                <MarketingAgent
                  directory={directory()}
                  context={context}
                  onClose={() => setState("agent", false)}
                  onIdle={() => {
                    refresh()
                    model.setState("analyticsRevision", (value) => value + 1)
                  }}
                />
              </Suspense>
            </Show>
          </Match>
        </Switch>
      </div>
    </div>
  )
}

function SignIn(props: { onRetry: () => void }): JSX.Element {
  const language = useLanguage()
  const dialog = useDialog()
  const connect = async () => {
    const { DialogConnectProvider, useProviderConnectController } = await import("@/providers/connect/dialog")
    const controller = useProviderConnectController()
    controller.select("cryzo")
    void dialog.show(() => (
      <DialogConnectProvider controller={controller} standalone onConnected={() => props.onRetry()} />
    ))
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

function localTime() {
  const now = new Date()
  const offset = -now.getTimezoneOffset()
  const pad = (value: number) => String(Math.floor(Math.abs(value))).padStart(2, "0")
  return `${new Date(now.getTime() + offset * 60_000).toISOString().slice(0, 19)}${offset < 0 ? "-" : "+"}${pad(offset / 60)}:${pad(offset % 60)}`
}
