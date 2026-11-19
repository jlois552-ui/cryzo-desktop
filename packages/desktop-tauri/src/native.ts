import { Channel, invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import type { ElectronNative } from "../../desktop/src/preload/types"
import type { BrowserPane } from "@opencode/app/desktop"
import { installNativeSockets } from "./native-socket"
import { installNativeChrome } from "./native-chrome"

type Bootstrap = { url: string; windowID: string; firstLaunchPending: boolean }
type DesktopEvent = { _tag: string } & Record<string, unknown>
type DesktopRequest = { _tag: string; id: number; tag: string; payload: unknown; requestId?: number }
type FetchEvent =
  | { kind: "head"; status: number; headers: [string, string][] }
  | { kind: "data"; bytes: number[] }
  | { kind: "end" }
  | { kind: "error"; message: string }

declare global {
  interface Window {
    cryzoPort: MessagePort
  }
}

export async function initializeNativeDesktop() {
  const bootstrap = await invoke<Bootstrap>("native_bootstrap")
  const channel = new MessageChannel()
  window.cryzoPort = channel.port2
  // The browser pane talks to its own Tauri command; the renderer platform picks it up from here.
  const pane = createBrowserPane()
  Object.assign(window, { cryzoBrowserPane: pane.api })
  // Keep the upstream renderer contract and route native operations through Tauri.
  // No Electron process, preload, or TypeScript coding engine is launched.
  window.electron = {
    windowID: bootstrap.windowID,
    bootstrap: { id: bootstrap.windowID, firstLaunchPending: bootstrap.firstLaunchPending, packaged: true },
    storageSnapshot: Promise.resolve({}),
    plugins: Promise.resolve([]),
    getPathForFile: () => "",
  } satisfies ElectronNative
  let events: number | undefined
  const caption = installNativeChrome()
  await listen<DesktopEvent>("cryzo:desktop", ({ payload }) => {
    caption(payload)
    pane.event(payload)
    if (events !== undefined) channel.port1.postMessage({ _tag: "Chunk", requestId: events, values: [payload] })
  })
  channel.port1.onmessage = ({ data }: MessageEvent<DesktopRequest>) => {
    if (data._tag === "Ack") return
    if (data._tag === "Interrupt") {
      void invoke("native_interrupt", { id: String(data.requestId) })
      return
    }
    if (data.tag === "DesktopEvents") {
      events = data.id
      return
    }
    const payload =
      data.tag === "DraftsPutBlob"
        ? { ...(data.payload as object), data: [...(data.payload as { data: Uint8Array }).data] }
        : data.payload
    void invoke("desktop_call", { tag: data.tag, payload }).then(
      (value) => {
        const result =
          ["DraftsGetBlob", "FilesReadPickedFile"].includes(data.tag) && Array.isArray(value)
            ? Uint8Array.from(value)
            : value
        channel.port1.postMessage({ _tag: "Exit", requestId: data.id, exit: { _tag: "Success", value: result } })
      },
      (error: unknown) =>
        channel.port1.postMessage({
          _tag: "Exit",
          requestId: data.id,
          exit: { _tag: "Failure", cause: [{ _tag: "Fail", error: nativeError(error) }] },
        }),
    )
  }
  channel.port1.start()
  const browserFetch = window.fetch.bind(window)
  const origin = new URL(bootstrap.url).origin
  installNativeSockets(origin)
  window.fetch = async (input, init) => {
    const request = new Request(input, init)
    if (new URL(request.url).origin !== origin) return browserFetch(request)
    return nativeFetch(request)
  }
}

async function nativeFetch(request: Request): Promise<Response> {
  if (request.signal.aborted) throw request.signal.reason
  const id = crypto.randomUUID()
  let controller: ReadableStreamDefaultController<Uint8Array> | undefined
  let finished = false
  let detach = () => {}
  const body = new ReadableStream<Uint8Array>({
    start(value) {
      controller = value
    },
    cancel() {
      finished = true
      detach()
      void invoke("native_interrupt", { id })
    },
  })
  return new Promise<Response>((resolve, reject) => {
    const abort = () => {
      void invoke("native_interrupt", { id })
      if (!finished) controller?.error(request.signal.reason)
      finished = true
      reject(request.signal.reason)
      detach()
    }
    request.signal.addEventListener("abort", abort, { once: true })
    const close = () => request.signal.removeEventListener("abort", abort)
    detach = close
    const channel = new Channel<FetchEvent>((event) => {
      if (finished) return
      if (event.kind === "head")
        resolve(
          new Response([204, 205, 304].includes(event.status) ? null : body, {
            status: event.status,
            headers: event.headers,
          }),
        )
      if (event.kind === "data") controller?.enqueue(Uint8Array.from(event.bytes))
      if (event.kind === "end") {
        finished = true
        controller?.close()
        close()
      }
      if (event.kind === "error") {
        finished = true
        const error = new Error(event.message)
        controller?.error(error)
        reject(error)
        close()
      }
    })
    void request
      .arrayBuffer()
      .then((bytes) => {
        if (request.signal.aborted) throw request.signal.reason
        return invoke("native_fetch", {
          request: {
            id,
            url: request.url,
            method: request.method,
            headers: [...request.headers],
            body: [...new Uint8Array(bytes)],
          },
          channel,
        })
      })
      .catch((error: unknown) => {
        if (finished) return
        finished = true
        controller?.error(error)
        reject(error)
        close()
      })
  })
}

function createBrowserPane() {
  const loading = new Set<(tab: string) => void>()
  const loaded = new Set<(tab: string, url: string) => void>()
  const titles = new Set<(tab: string, title: string) => void>()
  const newTabs = new Set<(url: string) => void>()
  const notes = new Set<(notes: readonly unknown[], screenshot?: string) => void>()
  const selections = new Set<(selection: unknown) => void>()
  const previews = new Set<(preview: { directory: string; url: string }) => void>()
  const call = (tag: string, payload: object = {}) => invoke<void>("browser_pane", { tag, payload })
  const listen = <T>(set: Set<T>) => (callback: T) => {
    set.add(callback)
    return () => void set.delete(callback)
  }
  const api: BrowserPane = {
    show: (tab, bounds, url) => call("BrowserShow", url ? { ...bounds, tab, url } : { ...bounds, tab }),
    closeTab: (tab) => call("BrowserTabClose", { tab }),
    bounds: (bounds) => call("BrowserBounds", bounds),
    hide: () => call("BrowserHide"),
    navigate: (url) => call("BrowserNavigate", { url }),
    back: () => call("BrowserBack"),
    forward: () => call("BrowserForward"),
    reload: () => call("BrowserReload"),
    still: () =>
      invoke<{ screenshot?: unknown }>("browser_pane", { tag: "BrowserStill", payload: {} }).then((reply) =>
        typeof reply?.screenshot === "string" ? reply.screenshot : undefined,
      ),
    annotate: (enabled, batch, labels) => call("BrowserAnnotate", { enabled, batch, labels }),
    sendBatch: () => call("BrowserSendBatch"),
    style: (property, value) => call("BrowserStyle", { property, value }),
    resetEdit: () => call("BrowserEditReset"),
    cancelEdit: () => call("BrowserEditCancel"),
    sendEdit: (comment) => call("BrowserEditSend", { comment }),
    onLoading: listen(loading),
    onLoaded: listen(loaded),
    onTitle: listen(titles),
    onNewTab: listen(newTabs),
    onNotes: listen(notes),
    onSelection: listen(selections),
    onPreview: listen(previews),
  }
  return {
    api,
    event(payload: DesktopEvent) {
      const tab = typeof payload.tab === "string" ? payload.tab : ""
      if (payload._tag === "BrowserLoading") loading.forEach((fn) => fn(tab))
      if (payload._tag === "BrowserLoaded" && typeof payload.url === "string") {
        const url = payload.url
        loaded.forEach((fn) => fn(tab, url))
      }
      if (payload._tag === "BrowserTitle" && typeof payload.title === "string") {
        const title = payload.title
        titles.forEach((fn) => fn(tab, title))
      }
      if (payload._tag === "BrowserNewTab" && typeof payload.url === "string") {
        const url = payload.url
        newTabs.forEach((fn) => fn(url))
      }
      if (payload._tag === "BrowserSelection") selections.forEach((fn) => fn(payload.selection))
      if (payload._tag === "BrowserNotes" && Array.isArray(payload.notes)) {
        const screenshot = typeof payload.screenshot === "string" ? payload.screenshot : undefined
        notes.forEach((fn) => fn(payload.notes as unknown[], screenshot))
      }
      if (payload._tag === "BrowserOpen" && typeof payload.url === "string" && typeof payload.directory === "string") {
        const preview = { directory: payload.directory, url: payload.url }
        previews.forEach((fn) => fn(preview))
      }
    },
  }
}

function nativeError(error: unknown) {
  if (typeof error !== "string") return error
  try {
    return JSON.parse(error) as unknown
  } catch {
    return error
  }
}
