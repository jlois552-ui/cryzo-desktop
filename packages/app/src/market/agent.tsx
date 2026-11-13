import { Icon } from "@opencode/ui/icon"
import { base64Encode } from "@opencode/util/encode"
import { IconButton } from "@opencode/ui/icon-button"
import { ResizeHandle } from "@opencode/ui/resize-handle"
import { Schema } from "effect"
import { For, Show, createEffect, createMemo } from "solid-js"
import { createStore } from "solid-js/store"
import { ResolvedDraftContent } from "@/new-session/route"
import { useLanguage } from "@/runtime/i18n/language"
import { persisted, Persist } from "@/runtime/persistence/storage"
import { Persistence } from "@/runtime/persistence/schema"
import { useData, useServer } from "@/runtime/server/current"
import { TargetSessionRouteContent } from "@/session/route"
import { SessionSurfaceProvider } from "@/session/surface"
import { useTabs, type DraftTab } from "@/shell/tabs/tabs"
import { MarketIcon } from "./icons"

const AgentState = Persistence.struct({
  session: Persistence.optional(Schema.String),
  draft: Persistence.optional(Schema.String),
  directory: Persistence.optional(Schema.String),
  width: Schema.Number,
})

const day = 86_400_000
type Bucket = "today" | "yesterday" | "week" | "month" | "older"
const buckets: Bucket[] = ["today", "yesterday", "week", "month", "older"]

/** ChatGPT-style recency buckets for the chat list. */
function bucket(updated: number, now = Date.now()): Bucket {
  const time = updated < 1e12 ? updated * 1000 : updated
  const midnight = new Date(now)
  midnight.setHours(0, 0, 0, 0)
  const start = midnight.getTime()
  if (time >= start) return "today"
  if (time >= start - day) return "yesterday"
  if (time >= start - 7 * day) return "week"
  if (time >= start - 30 * day) return "month"
  return "older"
}

/**
 * The Marketing agent panel, laid out like ChatGPT's side panel: a sidebar toggle that slides the chat list over
 * the conversation, the chat title, a new-chat button and close. Chats are the same sessions as coding chats.
 */
export function MarketingAgent(props: {
  directory?: string
  context: () => string
  onClose: () => void
  onIdle: () => void
}) {
  const language = useLanguage()
  const server = useServer()
  const data = useData()
  const tabs = useTabs()
  const [saved, save, , ready] = persisted(Persist.global(`market-agent-${server.key}`), AgentState, { width: 400 })
  const [state, setState] = createStore({ history: false, search: "", creating: false, wasRunning: false })
  let searchInput: HTMLInputElement | undefined
  const draft = createMemo(() =>
    tabs.store.find((tab): tab is DraftTab => tab.type === "draft" && tab.draftID === saved.draft),
  )
  const sessions = createMemo(() =>
    data.session
      .list()
      .filter(
        (session) =>
          !session.time.archived &&
          !session.parentID &&
          (session.title ?? "").toLowerCase().includes(state.search.trim().toLowerCase()),
      )
      .slice()
      .sort((a, b) => b.time.updated - a.time.updated),
  )
  const groups = createMemo(() => {
    const now = Date.now()
    const list = sessions()
    return buckets
      .map((name) => ({ name, items: list.filter((session) => bucket(session.time.updated, now) === name) }))
      .filter((group) => group.items.length)
  })
  const title = () =>
    (saved.session && data.session.get(saved.session)?.title) || language.t("market.agent.newChat")
  const create = async () => {
    const directory = saved.directory ?? props.directory
    if (!directory || state.creating) return
    setState("creating", true)
    try {
      const tab = await tabs.newDraft({ server: server.key, directory }, undefined, undefined, { activate: false })
      save({ session: undefined, draft: tab.draftID, directory })
      setState("history", false)
    } finally {
      setState("creating", false)
    }
  }
  createEffect(() => {
    if (!ready() || !tabs.ready() || saved.session || draft() || state.creating) return
    void create()
  })
  createEffect(() => {
    const running = !!saved.session && data.session.status(saved.session) === "running"
    if (state.wasRunning && !running) props.onIdle()
    if (state.wasRunning !== running) setState("wasRunning", running)
  })
  createEffect(() => {
    if (state.history) queueMicrotask(() => searchInput?.focus())
    else setState("search", "")
  })
  const select = (id: string) => {
    const session = data.session.get(id)
    if (!session) return
    save({ session: id, draft: undefined, directory: session.location.directory })
    setState("history", false)
    void data.session.sync(id)
  }
  const surface = {
    params: {
      get id() {
        return saved.session
      },
      get serverKey() {
        return base64Encode(server.key)
      },
    },
    draftId: () => saved.draft,
    started: (id: string) => select(id),
    newChat: create,
    context: props.context,
    navigate: (url: string | number) => {
      if (typeof url !== "string") return
      const match = url.match(/\/session\/([^/?#]+)/)
      if (match) select(decodeURIComponent(match[1]))
    },
  }
  const toggleLabel = () => language.t(state.history ? "market.agent.hideChats" : "market.agent.showChats")
  return (
    <section
      aria-label={language.t("market.agent.title")}
      class="market-agent relative flex h-full min-h-0 shrink-0 flex-col border-s border-v2-border-border-muted bg-v2-background-bg-base"
      style={{ "--market-agent-width": `${saved.width}px` }}
      onKeyDown={(event) => {
        if (event.key !== "Escape" || !state.history) return
        event.stopPropagation()
        setState("history", false)
      }}
    >
      <ResizeHandle
        direction="horizontal"
        edge="start"
        size={saved.width}
        min={320}
        max={720}
        onResize={(width) => save("width", Math.round(width))}
        aria-label={language.t("market.agent.width")}
      />
      <header class="flex h-12 shrink-0 items-center gap-1 px-2">
        <IconButton
          icon={<MarketIcon name="sidebar" class="size-4" />}
          variant="ghost-muted"
          aria-label={toggleLabel()}
          title={toggleLabel()}
          aria-expanded={state.history}
          onClick={() => setState("history", !state.history)}
        />
        <span
          class="min-w-0 flex-1 truncate px-1 text-[14px] font-medium text-v2-text-text-base"
          title={saved.directory ?? props.directory}
        >
          {title()}
        </span>
        <IconButton
          icon={<MarketIcon name="compose" class="size-4" />}
          variant="ghost-muted"
          aria-label={language.t("market.agent.newChat")}
          title={language.t("market.agent.newChat")}
          disabled={state.creating}
          onClick={() => void create()}
        />
        <IconButton
          icon={<Icon name="close" />}
          variant="ghost-muted"
          aria-label={language.t("common.close")}
          title={language.t("common.close")}
          onClick={props.onClose}
        />
      </header>
      <div class="relative flex min-h-0 flex-1 flex-col">
        <Show
          when={ready() && (saved.session || draft())}
          fallback={
            <div class="flex flex-1 items-center justify-center p-5 text-center text-[13px] text-v2-text-text-muted">
              {language.t("market.agent.projectRequired")}
            </div>
          }
        >
          <SessionSurfaceProvider value={surface}>
            <Show
              when={saved.session}
              fallback={
                <Show when={draft()} keyed>
                  {(tab) => <ResolvedDraftContent draft={tab} />}
                </Show>
              }
            >
              <TargetSessionRouteContent />
            </Show>
          </SessionSurfaceProvider>
        </Show>
      </div>
      <Show when={state.history}>
        <div class="market-agent-scrim absolute inset-0 z-20" onClick={() => setState("history", false)} />
        <nav
          aria-label={language.t("market.agent.history")}
          class="market-agent-drawer absolute inset-y-0 start-0 z-30 flex flex-col bg-v2-background-bg-layer-01 shadow-lg"
        >
          <div class="flex h-12 shrink-0 items-center gap-1 px-2">
            <span class="min-w-0 flex-1 truncate px-2 text-[14px] font-medium">{language.t("market.agent.chats")}</span>
            <IconButton
              icon={<MarketIcon name="sidebar" class="size-4" />}
              variant="ghost-muted"
              aria-label={language.t("market.agent.hideChats")}
              title={language.t("market.agent.hideChats")}
              onClick={() => setState("history", false)}
            />
          </div>
          <div class="flex shrink-0 flex-col gap-1 px-2 pb-2">
            <button
              type="button"
              class="flex h-9 items-center gap-2.5 rounded-lg px-2 text-start text-[13px] text-v2-text-text-base hover:bg-v2-overlay-simple-overlay-hover disabled:opacity-50"
              disabled={state.creating || !(saved.directory ?? props.directory)}
              onClick={() => void create()}
            >
              <MarketIcon name="compose" class="size-4 shrink-0" />
              <span class="truncate">{language.t("market.agent.newChat")}</span>
            </button>
            <label class="flex h-9 items-center gap-2.5 rounded-lg px-2 text-[13px] text-v2-text-text-muted focus-within:bg-v2-overlay-simple-overlay-hover hover:bg-v2-overlay-simple-overlay-hover">
              <Icon name="magnifying-glass" size="small" class="shrink-0" />
              <input
                ref={searchInput}
                aria-label={language.t("market.agent.search")}
                placeholder={language.t("market.agent.search")}
                value={state.search}
                onInput={(event) => setState("search", event.currentTarget.value)}
                class="min-w-0 flex-1 bg-transparent text-v2-text-text-base outline-none placeholder:text-v2-text-text-muted"
              />
            </label>
          </div>
          <div class="min-h-0 flex-1 overflow-y-auto px-2 pb-3" data-scrollable>
            <For
              each={groups()}
              fallback={
                <p class="px-2 py-6 text-[13px] text-v2-text-text-muted">
                  {language.t(state.search.trim() ? "market.agent.noResults" : "market.agent.empty")}
                </p>
              }
            >
              {(group) => (
                <div class="pt-3">
                  <div class="px-2 pb-1 text-[11px] font-medium text-v2-text-text-muted">
                    {language.t(`market.agent.group.${group.name}`)}
                  </div>
                  <For each={group.items}>
                    {(session) => (
                      <button
                        type="button"
                        aria-current={saved.session === session.id ? "page" : undefined}
                        title={session.location.directory}
                        class="flex h-9 w-full items-center rounded-lg px-2 text-start text-[13px] text-v2-text-text-base hover:bg-v2-overlay-simple-overlay-hover aria-[current=page]:bg-v2-background-bg-layer-03"
                        onClick={() => select(session.id)}
                      >
                        <span class="min-w-0 flex-1 truncate">
                          {session.title || language.t("market.agent.newChat")}
                        </span>
                      </button>
                    )}
                  </For>
                </div>
              )}
            </For>
          </div>
        </nav>
      </Show>
    </section>
  )
}
