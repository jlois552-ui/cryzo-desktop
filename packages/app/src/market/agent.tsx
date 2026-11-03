import { Icon } from "@opencode/ui/icon"
import { base64Encode } from "@opencode/util/encode"
import { IconButton } from "@opencode/ui/icon-button"
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

const AgentState = Persistence.struct({
  session: Persistence.optional(Schema.String),
  draft: Persistence.optional(Schema.String),
  directory: Persistence.optional(Schema.String),
  width: Schema.Number,
})

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
          (session.title ?? "").toLowerCase().includes(state.search.toLowerCase()),
      )
      .slice()
      .sort((a, b) => b.time.updated - a.time.updated),
  )
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
  return (
    <section
      aria-label={language.t("market.agent.title")}
      class="market-agent flex h-full min-h-0 shrink-0 flex-col border-s border-v2-border-border-muted bg-v2-background-bg-base"
      style={{ "--market-agent-width": `${saved.width}px` }}
    >
      <header class="flex h-12 shrink-0 items-center gap-1 border-b border-v2-border-border-muted px-3">
        <span class="min-w-0 flex-1 truncate text-[13px] font-medium">
          {saved.session ? data.session.get(saved.session)?.title : language.t("market.agent.title")}
        </span>
        <IconButton
          icon={<Icon name="plus" />}
          variant="ghost-muted"
          aria-label={language.t("command.session.new")}
          onClick={() => void create()}
        />
        <IconButton
          icon={<Icon name="history" />}
          variant="ghost-muted"
          aria-label={language.t("market.agent.history")}
          aria-expanded={state.history}
          onClick={() => setState("history", !state.history)}
        />
        <IconButton
          icon={<Icon name="close" />}
          variant="ghost-muted"
          aria-label={language.t("common.close")}
          onClick={props.onClose}
        />
      </header>
      <div class="flex items-center gap-2 border-b border-v2-border-border-muted px-3 py-2 text-[11px] text-v2-text-text-muted">
        <span class="min-w-0 flex-1 truncate" title={saved.directory ?? props.directory}>
          {saved.directory ?? props.directory}
        </span>
        <input
          aria-label={language.t("market.agent.width")}
          type="range"
          min="320"
          max="560"
          step="20"
          value={saved.width}
          onInput={(event) => save("width", Number(event.currentTarget.value))}
          class="w-16"
        />
      </div>
      <Show
        when={state.history}
        fallback={
          <Show
            when={ready() && (saved.session || draft())}
            fallback={
              <div class="flex flex-1 items-center justify-center p-5 text-[13px] text-v2-text-text-muted">
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
        }
      >
        <div class="flex min-h-0 flex-1 flex-col gap-2 p-3">
          <input
            aria-label={language.t("market.agent.history")}
            placeholder={language.t("market.agent.search")}
            value={state.search}
            onInput={(event) => setState("search", event.currentTarget.value)}
            class="h-9 rounded-lg border border-v2-border-border-base bg-v2-background-bg-base px-3 text-[13px]"
          />
          <div class="min-h-0 flex-1 overflow-y-auto" data-scrollable>
            <For each={sessions()}>
              {(session) => (
                <button
                  type="button"
                  class="flex w-full flex-col gap-1 rounded-lg p-3 text-start hover:bg-v2-overlay-simple-overlay-hover"
                  onClick={() => select(session.id)}
                >
                  <span class="line-clamp-2 text-[13px]">{session.title}</span>
                  <span class="max-w-full truncate text-[11px] text-v2-text-text-muted">
                    {session.location.directory}
                  </span>
                </button>
              )}
            </For>
          </div>
        </div>
      </Show>
    </section>
  )
}
