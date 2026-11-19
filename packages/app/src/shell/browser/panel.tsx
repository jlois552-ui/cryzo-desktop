import { createEffect, createRoot, For, on, onCleanup, onMount, Show } from "solid-js"
import { createStore, produce } from "solid-js/store"
import { resolveAddress } from "@opencode/gui-plugins/browser/address"
import { bare, completes, highlight, recordable, remember, suggest, type Visit } from "@opencode/gui-plugins/browser/history"
import { Icon } from "@opencode/ui/icon"
import { IconButton } from "@opencode/ui/icon-button"
import { ResizeHandle } from "@opencode/ui/resize-handle"
import { useLanguage } from "@/runtime/i18n/language"
import { type BrowserPane, usePlatform } from "@/runtime/platform/platform"
import { BrowserInspector } from "./inspector"
import { browserSelection, type BrowserSelection } from "./notes"

const lastUrlKey = "cryzo.browser.url"
const tabsKey = "cryzo.browser.tabs"
const historyKey = "cryzo.browser.history"
const inspectorWidth = 280

type Tab = { id: string; url: string; title: string; loading?: boolean }

/** Whether the browser pane is open and in select mode; shared by the shell, its commands and the pane. */
export const browserPanel = createRoot(() => {
  const [state, setState] = createStore({
    open: false,
    selecting: false,
    /** An address another part of the app asked the pane to show, such as a preview; `at` repeats a request. */
    request: undefined as { url: string; at: number } | undefined,
  })
  return {
    open: () => state.open,
    toggle: () => setState("open", (open) => !open),
    close: () => setState({ open: false, selecting: false }),
    request: () => state.request,
    openAt: (url: string) => setState({ open: true, request: { url, at: Date.now() } }),
    selecting: () => state.selecting,
    toggleSelect: () => setState({ open: true, selecting: !state.selecting }),
    setSelecting: (selecting: boolean) => setState("selecting", selecting),
    /** Tabs whose native page exists; it outlives the pane closing, which only hides it. */
    created: new Set<string>(),
  }
})

export function BrowserPanel() {
  const platform = usePlatform()
  return (
    <Show when={browserPanel.open() && platform.browserPane}>
      {(pane) => <Panel pane={pane()} />}
    </Show>
  )
}

function Panel(props: { pane: BrowserPane }) {
  const language = useLanguage()
  const platform = usePlatform()
  const saved = readTabs()
  const [state, setState] = createStore({
    tabs: saved.tabs,
    active: saved.active,
    input: saved.tabs.find((tab) => tab.id === saved.active)?.url ?? "",
    /** The address field has focus; `typed` once the user changed its text. */
    editing: false,
    typed: false,
    choice: -1,
    /** A picture of the page shown in its place while the suggestions overlap it. */
    still: undefined as string | undefined,
    batch: false,
    width: 520,
    selection: undefined as BrowserSelection | undefined,
    history: readHistory(),
  })
  let pane: HTMLElement | undefined
  let viewport: HTMLDivElement | undefined
  let address: HTMLInputElement | undefined

  // The native webview is placed over this element; its rect is in CSS pixels, the host wants window pixels.
  const bounds = () => {
    const rect = viewport!.getBoundingClientRect()
    const zoom = platform.webviewZoom?.() ?? 1
    return { x: rect.left * zoom, y: rect.top * zoom, width: rect.width * zoom, height: rect.height * zoom }
  }
  const labels = () => ({
    comment: language.t("browser.picker.comment"),
    send: language.t("browser.picker.send"),
    add: language.t("browser.picker.add"),
    edit: language.t("browser.picker.edit"),
  })
  const active = () => state.tabs.find((tab) => tab.id === state.active)
  const suggestions = (): Visit[] => {
    if (!state.editing) return []
    if (!state.typed || !state.input.trim()) return state.history.slice(0, 8)
    return suggest(state.history, state.input, 8)
  }

  // Shows a tab's native page; its first showing creates it at the tab's address.
  const reveal = async (tab: Tab) => {
    const create = !browserPanel.created.has(tab.id)
    browserPanel.created.add(tab.id)
    await props.pane.show(tab.id, bounds(), create ? tab.url : undefined)
    if (browserPanel.selecting()) await props.pane.annotate(true, state.batch, labels())
  }

  const select = (id: string) => {
    const tab = state.tabs.find((item) => item.id === id)
    if (!tab || id === state.active) return
    closeInspector()
    setState({ active: id, input: tab.url })
    saveTabs()
    void reveal(tab)
  }

  const openTab = (url: string) => {
    const tab = { id: crypto.randomUUID(), url, title: "" }
    closeInspector()
    setState(produce((draft) => {
      draft.tabs.push(tab)
      draft.active = tab.id
      draft.input = url === "about:blank" ? "" : url
    }))
    saveTabs()
    // A new blank tab starts in the address field, after its page has taken the window's focus.
    void reveal(tab).then(() => url === "about:blank" && address?.focus())
  }

  const closeTab = (id: string) => {
    const index = state.tabs.findIndex((tab) => tab.id === id)
    if (index < 0) return
    browserPanel.created.delete(id)
    void props.pane.closeTab(id)
    const rest = state.tabs.filter((tab) => tab.id !== id)
    if (!rest.length) {
      setState("tabs", [])
      saveTabs()
      return browserPanel.close()
    }
    setState("tabs", rest)
    if (id === state.active) {
      const next = rest[Math.min(index, rest.length - 1)]!
      setState({ active: next.id, input: next.url })
      closeInspector()
      void reveal(next)
    }
    saveTabs()
  }

  // A request from elsewhere in the app reuses a tab already on that origin, else opens one.
  // A restored tab has no native page until it is shown, so it is created at the address; a shown
  // tab navigates only once it is the pane's active page.
  const openRequested = async (url: string) => {
    const origin = originOf(url)
    const same = origin ? state.tabs.find((tab) => originOf(tab.url) === origin) : undefined
    if (!same) return openTab(url)
    if (same.id !== state.active) {
      closeInspector()
      setState({ active: same.id, input: url })
    }
    if (!browserPanel.created.has(same.id)) {
      setState("tabs", (tab) => tab.id === same.id, "url", url)
      saveTabs()
      return reveal({ ...same, url })
    }
    saveTabs()
    await reveal(same)
    await props.pane.navigate(url)
  }

  const go = (text: string) => {
    const url = resolveAddress(text)
    setState({ input: url === "about:blank" ? "" : url })
    address?.blur()
    void props.pane.navigate(url)
  }

  // The suggestions are drawn by the app, under the native page: while they show, a still of the page
  // takes its place.
  const startEditing = () => {
    setState({ editing: true, typed: false, choice: -1 })
    address?.select()
    void props.pane.still().then((shot) => {
      if (!state.editing || !shot || !document.hasFocus() || document.activeElement !== address) return
      setState("still", shot)
      void props.pane.hide()
    })
  }

  const stopEditing = () => {
    setState({ editing: false, typed: false, choice: -1 })
    if (document.activeElement !== address) setState("input", active()?.url === "about:blank" ? "" : (active()?.url ?? ""))
    if (!state.still) return
    setState("still", undefined)
    const tab = active()
    if (tab) void reveal(tab)
  }

  const closeInspector = () => {
    if (!state.selection) return
    setState({ selection: undefined, width: Math.max(320, state.width - inspectorWidth) })
  }

  onMount(() => {
    const request = browserPanel.request()
    if (!state.tabs.length) openTab(request?.url ?? readUrl())
    else if (request) void openRequested(request.url)
    else void reveal(active()!)
    // The page area also moves without resizing, as when the inspector opens beside it and widens the pane.
    const observer = new ResizeObserver(() => void props.pane.bounds(bounds()))
    observer.observe(viewport!)
    observer.observe(pane!)
    const offs = [
      props.pane.onLoading((tab) => setState("tabs", (item) => item.id === tab, "loading", true)),
      props.pane.onLoaded((tab, url) => {
        const moved = state.tabs.find((item) => item.id === tab)?.url !== url
        setState("tabs", (item) => item.id === tab, { url, loading: false })
        saveTabs()
        if (tab !== state.active) return
        // A page finishing its load must not overwrite an address the user is still typing.
        if (document.activeElement !== address) setState("input", url === "about:blank" ? "" : url)
        saveUrl(url)
        if (recordable(url)) {
          const title = state.tabs.find((item) => item.id === tab)?.title ?? ""
          setState("history", remember(state.history, { url, title }))
          saveHistory(state.history)
        }
        // A reloaded page starts without the picker switched on; a new address also drops the edited element.
        if (browserPanel.selecting()) void props.pane.annotate(true, state.batch, labels())
        if (moved) closeInspector()
      }),
      props.pane.onTitle((tab, title) => {
        setState("tabs", (item) => item.id === tab, "title", title.slice(0, 200))
        saveTabs()
        // A title that arrives while a page loads belongs to the next address, which the load reports.
        const current = state.tabs.find((item) => item.id === tab)
        const url = current?.url
        if (!url || current.loading || !state.history.some((visit) => visit.url === url)) return
        setState("history", remember(state.history, { url, title }))
        saveHistory(state.history)
      }),
      props.pane.onNewTab((url) => openTab(url)),
      props.pane.onSelection((raw) => {
        const selection = browserSelection(raw)
        if (!selection) return
        setState({ selection, width: state.selection ? state.width : state.width + inspectorWidth })
      }),
    ]
    onCleanup(() => {
      observer.disconnect()
      offs.forEach((off) => off())
      if (state.selection) void props.pane.cancelEdit()
      void props.pane.annotate(false, false, labels())
      void props.pane.hide()
    })
  })

  createEffect(on(() => platform.webviewZoom?.(), () => void props.pane.bounds(bounds()), { defer: true }))

  // A later request while the pane is open opens it in a tab.
  createEffect(
    on(
      browserPanel.request,
      (request) => {
        if (request) void openRequested(request.url)
      },
      { defer: true },
    ),
  )

  // The page's picker follows select mode, from the toolbar or the command.
  createEffect(
    on(
      () => [browserPanel.selecting(), state.batch] as const,
      ([selecting, batch]) => void props.pane.annotate(selecting, batch, labels()),
      { defer: true },
    ),
  )

  return (
    <aside
      ref={pane}
      data-slot="browser-pane"
      aria-label={language.t("browser.title")}
      class="relative flex h-full min-h-0 shrink-0 flex-col gap-1.5 py-[var(--shell-top-inset,8px)] pe-[var(--shell-inline-inset,8px)] pb-[var(--shell-bottom-inset,8px)]"
      style={{ width: `${state.width}px` }}
    >
      <ResizeHandle
        class="-start-1"
        direction="horizontal"
        edge="start"
        size={state.width}
        min={320}
        max={1600}
        onResize={(width) => setState("width", width)}
      />
      <div role="tablist" class="flex h-8 shrink-0 items-center gap-1 overflow-x-auto no-scrollbar">
        <For each={state.tabs}>
          {(tab) => (
            <div
              role="tab"
              aria-selected={tab.id === state.active}
              class="group flex h-7 min-w-0 max-w-[180px] flex-1 cursor-default items-center gap-1 rounded-[6px] ps-2 pe-0.5 text-12-regular"
              classList={{
                "bg-v2-background-bg-base text-v2-text-text-base": tab.id === state.active,
                "text-v2-text-text-muted hover:bg-v2-background-bg-base/60": tab.id !== state.active,
              }}
              onClick={() => select(tab.id)}
              onAuxClick={(event) => event.button === 1 && closeTab(tab.id)}
            >
              <span class="min-w-0 flex-1 truncate leading-[var(--line-height-compact)]">
                {tab.title || hostOf(tab.url) || language.t("browser.newTab")}
              </span>
              <IconButton
                icon={<Icon name="close" />}
                variant="ghost-muted"
                size="small"
                aria-label={language.t("browser.closeTab")}
                onClick={(event) => {
                  event.stopPropagation()
                  closeTab(tab.id)
                }}
              />
            </div>
          )}
        </For>
        <IconButton
          icon={<Icon name="plus" />}
          variant="ghost-muted"
          size="small"
          aria-label={language.t("browser.newTab")}
          onClick={() => openTab("about:blank")}
        />
      </div>
      <div class="flex h-8 shrink-0 items-center gap-1 rounded-[8px] bg-v2-background-bg-base px-1">
        <IconButton
          icon={<Icon name="outline-arrow-left" />}
          variant="ghost-muted"
          size="small"
          aria-label={language.t("browser.back")}
          onClick={() => void props.pane.back()}
        />
        <IconButton
          icon={<Icon name="outline-arrow-right" />}
          variant="ghost-muted"
          size="small"
          aria-label={language.t("browser.forward")}
          onClick={() => void props.pane.forward()}
        />
        <IconButton
          icon={<Icon name="outline-rotate-clockwise" />}
          variant="ghost-muted"
          size="small"
          aria-label={language.t("browser.reload")}
          onClick={() => void props.pane.reload()}
        />
        <form
          class="relative flex min-w-0 flex-1"
          onSubmit={(event) => {
            event.preventDefault()
            const picked = suggestions()[state.choice]
            if (picked) return go(picked.url)
            // Read the field itself: assistive tools can set its value without an input event.
            const text = String(new FormData(event.currentTarget).get("address") ?? "").trim()
            if (text) go(text)
          }}
        >
          <input
            class="h-6 min-w-0 flex-1 rounded-[6px] border border-v2-border-border-base bg-v2-background-bg-base px-2 font-mono text-12-regular text-v2-text-text-base"
            ref={address}
            name="address"
            role="combobox"
            autocomplete="off"
            aria-autocomplete="list"
            aria-expanded={suggestions().length > 0}
            aria-controls="browser-suggestions"
            value={state.input}
            placeholder={language.t("browser.address.placeholder")}
            aria-label={language.t("browser.address")}
            spellcheck={false}
            onFocus={() => (!state.editing || !state.still) && startEditing()}
            // Showing or hiding a native page moves the window's focus, which blurs the field without the user
            // leaving it; only a blur within the window ends the edit.
            onBlur={() => document.hasFocus() && stopEditing()}
            onInput={(event) => {
              const value = event.currentTarget.value
              const first = suggest(state.history, value, 1)[0]
              const deleting = event instanceof InputEvent && event.inputType.startsWith("delete")
              setState({ input: value, typed: true, choice: first && !deleting && completes(first, value) ? 0 : -1 })
            }}
            onKeyDown={(event) => {
              if (event.isComposing) return
              const count = suggestions().length
              if ((event.key === "ArrowDown" || event.key === "ArrowUp") && count) {
                event.preventDefault()
                // -1 is the typed text itself, between the last suggestion and the first.
                const step = event.key === "ArrowDown" ? 1 : -1
                setState("choice", (choice) => ((choice + 1 + step + count + 1) % (count + 1)) - 1)
                return
              }
              if (event.key === "Escape") {
                event.preventDefault()
                event.currentTarget.blur()
              }
            }}
          />
          <Show when={suggestions().length}>
            <div
              id="browser-suggestions"
              role="listbox"
              aria-label={language.t("browser.suggestions")}
              class="absolute inset-x-0 top-full z-20 mt-1 flex flex-col overflow-hidden rounded-[8px] bg-v2-background-bg-base p-1 shadow-[var(--v2-elevation-raised)]"
            >
              <For each={suggestions()}>
                {(visit, index) => (
                  <div
                    role="option"
                    aria-selected={index() === state.choice}
                    class="flex h-7 min-w-0 cursor-default items-center gap-1.5 rounded-[6px] px-2 text-12-regular"
                    classList={{ "bg-v2-background-bg-deep": index() === state.choice }}
                    dir="ltr"
                    // The field keeps focus, so a click does not end editing before it opens the page.
                    onPointerDown={(event) => event.preventDefault()}
                    onPointerEnter={() => setState("choice", index())}
                    onClick={() => go(visit.url)}
                  >
                    <Show when={visit.title}>
                      <span class="min-w-0 shrink truncate text-v2-text-text-base">
                        <Marked text={visit.title} query={state.typed ? state.input : ""} />
                      </span>
                      <span class="text-v2-text-text-muted">-</span>
                    </Show>
                    <span class="min-w-0 flex-1 truncate font-mono text-v2-text-text-muted">
                      <Marked text={bare(visit.url)} query={state.typed ? state.input : ""} />
                    </span>
                  </div>
                )}
              </For>
            </div>
          </Show>
        </form>
        <IconButton
          icon={<Icon name="outline-browser-annotate" />}
          variant={browserPanel.selecting() ? "neutral" : "ghost-muted"}
          size="small"
          aria-pressed={browserPanel.selecting()}
          aria-label={language.t("browser.annotate")}
          onClick={() => browserPanel.toggleSelect()}
        />
        <Show when={browserPanel.selecting()}>
          <IconButton
            icon={<Icon name={state.batch ? "checklist" : "bullet-list"} />}
            variant={state.batch ? "neutral" : "ghost-muted"}
            size="small"
            aria-pressed={state.batch}
            aria-label={language.t("browser.batch")}
            onClick={() => setState("batch", (batch) => !batch)}
          />
          <Show when={state.batch}>
            <IconButton
              icon={<Icon name="enter" />}
              variant="ghost-muted"
              size="small"
              aria-label={language.t("browser.sendBatch")}
              onClick={() => void props.pane.sendBatch()}
            />
          </Show>
        </Show>
        <IconButton
          icon={<Icon name="close" />}
          variant="ghost-muted"
          size="small"
          aria-label={language.t("browser.close")}
          onClick={() => browserPanel.close()}
        />
      </div>
      <div class="flex min-h-0 flex-1 gap-1.5">
        {/* The native page is drawn over this area by the desktop host. */}
        <div ref={viewport} class="relative min-h-0 min-w-0 flex-1 overflow-hidden rounded-[8px] bg-v2-background-bg-base">
          <Show when={state.still}>
            {(still) => <img alt="" class="absolute inset-0 size-full" src={`data:image/png;base64,${still()}`} />}
          </Show>
        </div>
        <Show when={state.selection} keyed>
          {(selection) => (
            <BrowserInspector
              selection={selection}
              width={inspectorWidth}
              onStyle={(property, value) => void props.pane.style(property, value)}
              onReset={() => void props.pane.resetEdit()}
              onCancel={() => {
                void props.pane.cancelEdit()
                closeInspector()
              }}
              onSend={(comment) => {
                void props.pane.sendEdit(comment)
                closeInspector()
              }}
            />
          )}
        </Show>
      </div>
    </aside>
  )

  function saveTabs() {
    writeStorage(tabsKey, JSON.stringify({ tabs: state.tabs, active: state.active }))
  }
}

function Marked(props: { text: string; query: string }) {
  return (
    <For each={highlight(props.text, props.query)}>
      {(part) => (
        <Show when={part.match} fallback={part.text}>
          <mark class="bg-transparent font-medium text-v2-text-text-base">{part.text}</mark>
        </Show>
      )}
    </For>
  )
}

function originOf(url: string) {
  return URL.canParse(url) ? new URL(url).origin : undefined
}

function hostOf(url: string) {
  return URL.canParse(url) && /^https?:$/.test(new URL(url).protocol) ? new URL(url).host : ""
}

// Tabs, history and the last address are this device's conveniences; the pane works without them.
function readTabs(): { tabs: Tab[]; active: string } {
  const value = parse(readStorage(tabsKey))
  const tabs = isRecord(value) && Array.isArray(value.tabs) ? value.tabs.flatMap(readTab).slice(0, 20) : []
  const active = isRecord(value) && typeof value.active === "string" ? value.active : ""
  return { tabs, active: tabs.some((tab) => tab.id === active) ? active : (tabs[0]?.id ?? "") }
}

function readTab(value: unknown): Tab[] {
  if (!isRecord(value) || typeof value.id !== "string" || !/^[a-zA-Z0-9-]{1,64}$/.test(value.id)) return []
  if (typeof value.url !== "string") return []
  return [{ id: value.id, url: value.url, title: typeof value.title === "string" ? value.title : "" }]
}

function readHistory(): Visit[] {
  const value = parse(readStorage(historyKey))
  return Array.isArray(value)
    ? value.flatMap((item) =>
        isRecord(item) && typeof item.url === "string" && typeof item.title === "string" ? [{ url: item.url, title: item.title }] : [],
      )
    : []
}

function saveHistory(visits: readonly Visit[]) {
  writeStorage(historyKey, JSON.stringify(visits.map((visit) => ({ url: visit.url, title: visit.title }))))
}

function readUrl() {
  return readStorage(lastUrlKey) || "http://localhost:5173"
}

function saveUrl(url: string) {
  if (url.startsWith("http")) writeStorage(lastUrlKey, url)
}

function parse(text: string | null) {
  if (!text) return undefined
  try {
    return JSON.parse(text) as unknown
  } catch {
    return undefined
  }
}

function readStorage(key: string) {
  try {
    return localStorage.getItem(key)
  } catch {
    return null
  }
}

function writeStorage(key: string, value: string) {
  try {
    localStorage.setItem(key, value)
  } catch {
    // Storage may be unavailable; the pane then starts fresh next time.
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value)
}
