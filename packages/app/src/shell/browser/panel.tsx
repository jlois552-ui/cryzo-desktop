import { createEffect, createRoot, on, onCleanup, onMount, Show } from "solid-js"
import { createStore } from "solid-js/store"
import { Icon } from "@opencode/ui/icon"
import { IconButton } from "@opencode/ui/icon-button"
import { ResizeHandle } from "@opencode/ui/resize-handle"
import { useLanguage } from "@/runtime/i18n/language"
import { type BrowserPane, usePlatform } from "@/runtime/platform/platform"

const lastUrlKey = "cryzo.browser.url"

/** Whether the browser pane is open; shared by the shell, the sidebar row and the command. */
export const browserPanel = createRoot(() => {
  const [state, setState] = createStore({ open: false, request: "" })
  return {
    open: () => state.open,
    toggle: () => setState("open", (open) => !open),
    close: () => setState("open", false),
    /** An address another part of the app asked the pane to show, such as a cloud preview. */
    request: () => state.request,
    openAt: (url: string) => setState({ open: true, request: url }),
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
  const start = browserPanel.request() || readUrl()
  const [state, setState] = createStore({ url: start, input: start, annotate: false, batch: false, draw: false, width: 520 })
  let viewport: HTMLDivElement | undefined
  let address: HTMLInputElement | undefined

  // The native webview is placed over this element; its rect is in CSS pixels, the host wants window pixels.
  const bounds = () => {
    const rect = viewport!.getBoundingClientRect()
    const zoom = platform.webviewZoom?.() ?? 1
    return { x: rect.left * zoom, y: rect.top * zoom, width: rect.width * zoom, height: rect.height * zoom }
  }

  onMount(() => {
    void props.pane.show(bounds(), state.url)
    const observer = new ResizeObserver(() => void props.pane.bounds(bounds()))
    observer.observe(viewport!)
    const offLoaded = props.pane.onLoaded((url) => {
      setState("url", url)
      // A page finishing its load must not overwrite an address the user is still typing.
      if (document.activeElement !== address) setState("input", url)
      saveUrl(url)
      // A new page starts without the picker switched on.
      if (state.annotate) void props.pane.annotate(true, state.batch, state.draw)
    })
    onCleanup(() => {
      observer.disconnect()
      offLoaded()
      void props.pane.annotate(false, false, false)
      void props.pane.hide()
    })
  })

  createEffect(on(() => platform.webviewZoom?.(), () => void props.pane.bounds(bounds()), { defer: true }))

  // A later request while the pane is open navigates it.
  createEffect(
    on(
      browserPanel.request,
      (url) => {
        if (url) void props.pane.navigate(url)
      },
      { defer: true },
    ),
  )

  const annotate = (enabled: boolean, batch = state.batch, draw = state.draw) => {
    setState({ annotate: enabled, batch, draw })
    void props.pane.annotate(enabled, batch, draw)
  }

  return (
    <aside
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
        max={1400}
        onResize={(width) => setState("width", width)}
      />
      <div class="flex h-8 shrink-0 items-center gap-1 rounded-[8px] bg-v2-background-bg-base px-1">
        <IconButton
          icon={<Icon name="arrow-left" />}
          variant="ghost-muted"
          size="small"
          aria-label={language.t("browser.back")}
          onClick={() => void props.pane.back()}
        />
        <IconButton
          icon={<Icon name="reset" />}
          variant="ghost-muted"
          size="small"
          aria-label={language.t("browser.reload")}
          onClick={() => void props.pane.reload()}
        />
        <form
          class="flex min-w-0 flex-1"
          onSubmit={(event) => {
            event.preventDefault()
            // Read the field itself: assistive tools can set its value without an input event.
            const url = String(new FormData(event.currentTarget).get("address") ?? "").trim()
            if (!url) return
            setState("input", url)
            address?.blur()
            void props.pane.navigate(url)
          }}
        >
          <input
            class="h-6 min-w-0 flex-1 rounded-[6px] border border-v2-border-border-base bg-v2-background-bg-base px-2 font-mono text-12-regular text-v2-text-text-base"
            ref={address}
            name="address"
            value={state.input}
            aria-label={language.t("browser.address")}
            spellcheck={false}
            onInput={(event) => setState("input", event.currentTarget.value)}
          />
        </form>
        <IconButton
          icon={<Icon name="window-cursor" />}
          variant={state.annotate ? "neutral" : "ghost-muted"}
          size="small"
          aria-pressed={state.annotate}
          aria-label={language.t("browser.annotate")}
          onClick={() => annotate(!state.annotate)}
        />
        <Show when={state.annotate}>
          <IconButton
            icon={<Icon name="edit" />}
            variant={state.draw ? "neutral" : "ghost-muted"}
            size="small"
            aria-pressed={state.draw}
            aria-label={language.t("browser.draw")}
            onClick={() => annotate(true, state.batch, !state.draw)}
          />
          <IconButton
            icon={<Icon name={state.batch ? "checklist" : "bullet-list"} />}
            variant={state.batch ? "neutral" : "ghost-muted"}
            size="small"
            aria-pressed={state.batch}
            aria-label={language.t("browser.batch")}
            onClick={() => annotate(true, !state.batch)}
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
      {/* The native page is drawn over this area by the desktop host. */}
      <div ref={viewport} class="min-h-0 flex-1 rounded-[8px] bg-v2-background-bg-base" />
    </aside>
  )
}

function readUrl() {
  try {
    return localStorage.getItem(lastUrlKey) || "http://localhost:5173"
  } catch {
    return "http://localhost:5173"
  }
}

function saveUrl(url: string) {
  try {
    if (url.startsWith("http")) localStorage.setItem(lastUrlKey, url)
  } catch {
    // Storage may be unavailable; the pane then opens at the default address.
  }
}
