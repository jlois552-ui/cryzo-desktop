import { createSimpleContext } from "@opencode/ui/context"
import type { AsyncStorage, SyncStorage } from "@solid-primitives/storage"
import type { Accessor } from "solid-js"
import type { DesktopMenuAction, DesktopZoomLevel } from "@/shell/commands/desktop-menu"
import type { DraftStore } from "@/runtime/persistence/drafts"
import type { Bridge } from "@opencode/plugin/gui/host"

type PickerPaths = string | string[] | null

type OpenDirectoryPickerOptions = { title?: string; multiple?: boolean }

type OpenAttachmentPickerOptions = {
  title?: string
  multiple?: boolean
  accept?: string[]
  extensions?: string[]
  defaultPath?: string
}

type OpenFilePathPickerOptions = { title?: string; multiple?: boolean; extensions?: string[] }

type SaveFilePickerOptions = { title?: string; defaultPath?: string }

type PlatformName = "web" | "desktop"

type DesktopOS = "macos" | "windows" | "linux"

/** Computer use (desktop control) settings, shared with Cryzo's engine and its MCP server. */
export type ComputerUseSettings = {
  enabled: boolean
  /** When agents ask before acting: every action, once per app in each session, or never. */
  approval: "always" | "per_app" | "never"
  typeIntoPasswords: boolean
  blockedApps: readonly string[]
}

/** Remote control: paired phones and browsers drive this computer's sessions through the Cryzo relay. */
export type RemoteControlStatus = {
  enabled: boolean
  connected: boolean
  /** Paired devices connected right now. */
  clients: number
  error: string | null
}

export type RemotePairing = {
  /** Typed on the phone, e.g. "K7QF-2M9X"; single use, expires at `expiresAt` (ms). */
  code: string
  expiresAt: number
  /** Opens the web client with the code filled in; null until the web client is deployed. */
  url: string | null
}

export type RemoteDevice = {
  id: string
  name: string
  createdAt: number
  lastSeenAt: number | null
}

/** A rectangle in the window's CSS pixels. */
export type BrowserPaneBounds = { x: number; y: number; width: number; height: number }

/** Words the page's picker shows, in the app's language. */
export type BrowserPickerLabels = { comment: string; send: string; add: string; edit: string }

/**
 * The desktop's browser pane: one native webview per tab, drawn over the area the app reserves for
 * it. Calls without a tab act on the tab shown last.
 */
export type BrowserPane = {
  /** Shows tab `tab`, creating it at `url` on first use or navigating it there, and hides the others. */
  show(tab: string, bounds: BrowserPaneBounds, url?: string): Promise<void>
  closeTab(tab: string): Promise<void>
  bounds(bounds: BrowserPaneBounds): Promise<void>
  hide(): Promise<void>
  navigate(url: string): Promise<void>
  back(): Promise<void>
  forward(): Promise<void>
  reload(): Promise<void>
  /** A base64 PNG of the shown page, drawn in its place while a menu overlaps the native page. */
  still(): Promise<string | undefined>
  /** Select mode: picks become notes; in batch mode they wait as numbered pins until `sendBatch`. */
  annotate(enabled: boolean, batch: boolean, labels: BrowserPickerLabels): Promise<void>
  sendBatch(): Promise<void>
  /** Previews a style (or `text`) change on the element being edited; the page ignores other properties. */
  style(property: string, value: string): Promise<void>
  /** Undoes every previewed change on the element being edited. */
  resetEdit(): Promise<void>
  /** Undoes the previewed changes and stops editing. */
  cancelEdit(): Promise<void>
  /** Sends the edited element as a note with its changes; the page then reverts them. */
  sendEdit(comment: string): Promise<void>
  /** A tab started loading a page; its title may change before the load finishes. */
  onLoading(callback: (tab: string) => void): () => void
  onLoaded(callback: (tab: string, url: string) => void): () => void
  onTitle(callback: (tab: string, title: string) => void): () => void
  /** A page asked for a new window, such as a target=_blank link; the app opens it as a tab. */
  onNewTab(callback: (url: string) => void): () => void
  /** Raw notes from the page's picker, as text; validate before use. */
  onNotes(callback: (notes: readonly unknown[]) => void): () => void
  /** The raw element the user chose to edit, validate before use. */
  onSelection(callback: (selection: unknown) => void): () => void
  /** The agent started a project's dev server and it answers at `url`. */
  onPreview(callback: (preview: { directory: string; url: string }) => void): () => void
}

export type FatalRendererErrorLog = {
  error: string
  url: string
  version?: string
  platform: PlatformName
  os?: DesktopOS
}

type PlatformBase = {
  requestMicrophone?(): Promise<boolean>
  setVoiceCallActive?(active: boolean): Promise<void>
  /** App version */
  version?: string

  /** Open a web or mail URL in the default system application */
  openExternal(url: string): void

  /** Open an authentication page, reporting whether the browser could be launched. */
  openBrowser?(url: string): Promise<boolean>

  /** Open a local path in a local app (desktop only) */
  openPath?(path: string, app?: string): Promise<void>

  /** Open a local file URL in its default app (desktop only) */
  openLocalFile?(url: string): void

  /** Reveal a local path in the system file manager; false when the path does not exist (desktop only) */
  revealPath?(path: string): Promise<boolean>

  /** Restart the app  */
  restart(): Promise<void>

  /** Send a system notification */
  notify(title: string, description?: string, onClick?: () => void): Promise<void>

  /** Open a native attachment picker and read selected files sequentially (desktop only) */
  openAttachmentPickerDialog?<Value>(
    opts: OpenAttachmentPickerOptions,
    onFile: (file: File) => Promise<Value>,
  ): Promise<void>

  /** Open a native file picker and return the selected paths without reading the files (desktop only) */
  openFilePathPickerDialog?(opts: OpenFilePathPickerOptions): Promise<string[] | null>

  /** Resolve the native source path for a desktop File. */
  getPathForFile?(file: File): string

  /** Observe native drag cancellation that does not reach the renderer event loop. */
  onDragCancel?(callback: () => void): () => void

  /** Open a native save file dialog and write content to the selected path (desktop only) */
  saveFile?(opts: SaveFilePickerOptions, content: string): Promise<boolean>

  /** Storage mechanism, defaults to localStorage */
  storage?: (name?: string) => SyncStorage | AsyncStorage

  /** Prompt drafts, history, and their blobs. */
  draftStore?: DraftStore

  /** Fetch override */
  fetch?: typeof fetch

  /** Webview zoom level (desktop only) */
  webviewZoom?: Accessor<number>

  /** Saved interface zoom level; a pinch can move `webviewZoom` off it (desktop only) */
  zoomLevel?: Accessor<DesktopZoomLevel>

  /** Save an interface zoom level and apply it to every window (desktop only) */
  setZoomLevel?(level: DesktopZoomLevel): Promise<void>

  /** Whether the native desktop window is fullscreen */
  windowFullscreen?: Accessor<boolean>

  /** Move the native macOS traffic lights' top-left corner, in window points (desktop only) */
  setTrafficLightsPosition?(position: { x: number; y: number }): Promise<void>

  /** Get whether native pinch/Ctrl-scroll zoom gestures are enabled (desktop only) */
  getPinchZoomEnabled?(): Promise<boolean> | boolean

  /** Allow native pinch/Ctrl-scroll zoom gestures (desktop only) */
  setPinchZoomEnabled?(enabled: boolean): Promise<void> | void

  /** Read computer use settings; null where the host has no computer use (desktop only) */
  getComputerUse?(): Promise<ComputerUseSettings | null>

  /** Save computer use settings; agents use them from their next action (desktop only) */
  setComputerUse?(settings: ComputerUseSettings): Promise<void>

  /** Read remote control state; null where the host has no remote control (desktop only) */
  getRemoteControl?(): Promise<RemoteControlStatus | null>

  /** Turn remote control on or off; off removes every paired device (desktop only) */
  setRemoteControl?(enabled: boolean): Promise<RemoteControlStatus>

  /** A new single-use pairing code for a phone or browser (desktop only) */
  pairRemoteDevice?(): Promise<RemotePairing>

  /** Devices paired with this computer (desktop only) */
  listRemoteDevices?(): Promise<RemoteDevice[]>

  /** Remove a paired device; its open connection is dropped (desktop only) */
  revokeRemoteDevice?(id: string): Promise<void>

  /** The built-in browser pane (Cryzo desktop only) */
  browserPane?: BrowserPane

  /** Run a desktop-only menu action from the app chrome */
  runDesktopMenuAction?(action: DesktopMenuAction): Promise<void> | void

  /** Check if an editor app exists (desktop only) */
  checkAppExists?(appName: string): Promise<boolean>

  /** Read image from clipboard (desktop only) */
  readClipboardImage?(): Promise<File | null>

  /** Write text to the native clipboard (desktop only) */
  writeClipboardText?(text: string): Promise<void>

  /** Export collected diagnostic logs (desktop only) */
  exportDebugLogs?(): Promise<string>

  /** Force focus styles on interactive elements through desktop devtools (desktop only) */
  setForceFocus?(enabled: boolean): Promise<void>

  /** Record a fatal renderer error in platform logs (desktop only) */
  recordFatalRendererError?(error: FatalRendererErrorLog): Promise<void>

  /** GUI plugin bridge to the main-process plugin host (desktop only). */
  plugins?: Bridge
}

export type Platform = PlatformBase &
  (
    | { platform: "web"; os?: never; windowID?: never }
    | {
        platform: "desktop"
        os?: DesktopOS
        /** Stable platform window identity for window-scoped persistence */
        windowID: string
        openDirectoryPickerDialog(opts?: OpenDirectoryPickerOptions): Promise<PickerPaths>
      }
  )

export const { use: usePlatform, provider: PlatformProvider } = createSimpleContext({
  name: "Platform",
  init: (props: { value: Platform }) => {
    return props.value
  },
})
