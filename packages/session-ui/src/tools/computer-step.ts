import type { UiI18n } from "@opencode/ui/context/i18n"

// SAFETY: tool input is an open per-tool record; each value is checked before it is read.
// oxlint-disable-next-line anti-slop/no-unsafe-dictionary-type -- see SAFETY above
type Input = Record<string, unknown>

/**
 * A desktop call's own arguments. The engine stores a finished call as `{tool: "desktop", command, args}`, while
 * a call still streaming in has the arguments alone.
 */
export function computerArgs(input: Input | undefined): Input {
  const args = input?.args
  if (input?.tool === "desktop" && args && typeof args === "object" && !Array.isArray(args)) return args as Input
  return input ?? {}
}

function text(value: unknown) {
  return typeof value === "string" ? value.trim() : ""
}

function clip(value: string, length = 40) {
  return value.length > length ? `${value.slice(0, length - 1)}…` : value
}

/** The window a result names, from its header: `window [w1a2b3c] "Title" (app.exe)`. */
function windowTitle(output: string | undefined) {
  return output?.match(/window \[w[0-9a-z]+\] "([^"]+)"/)?.[1]
}

/** The element an act result names, such as `click [e12] button "Send": done`. */
function elementName(output: string | undefined) {
  const match = output?.match(/\[[eb]\d+\] [\w-]+ "([^"]*)"/)
  return match?.[1] || undefined
}

/** `{ctrl}s` → `Ctrl+S`, `{enter}` → `Enter`. Plain text is never shown, as it may be something the user typed. */
function keys(value: string) {
  const named = [...value.matchAll(/\{([a-z0-9]+)(?: \d+)?\}/gi)].map((match) => match[1])
  if (named.length === 0) return undefined
  const rest = value.replace(/\{[^}]*\}/g, "").replace(/[()]/g, "")
  const all = [...named.map((name) => name[0].toUpperCase() + name.slice(1)), ...(rest.length <= 2 ? rest.toUpperCase() : [])]
  return all.filter(Boolean).join("+")
}

function host(url: string) {
  const match = url.match(/^[a-z]+:\/\/([^/?#]+)/i)
  return match?.[1] ?? clip(url)
}

/** One line saying what a computer-use call did, in the words a person would use. */
export function computerStep(i18n: UiI18n, name: string, input: Input | undefined, output: string | undefined) {
  const args = computerArgs(input)
  const command = name.replace(/^desktop_/, "")
  const app = clip(text(args.window) && !/^w[0-9a-z]+$/.test(text(args.window)) ? text(args.window) : (windowTitle(output) ?? ""))
  const target = clip(elementName(output) ?? text(args.name) ?? "") || i18n.t("ui.computer.element")
  const action = text(args.action)

  switch (command) {
    case "snapshot":
      return app ? i18n.t("ui.computer.read", { app }) : i18n.t("ui.computer.readScreen")
    case "windows":
      return i18n.t("ui.computer.windows")
    case "find":
      return i18n.t("ui.computer.find", { text: clip(text(args.text)) })
    case "wait_for":
      return i18n.t("ui.computer.waitFor", { text: clip(text(args.text)) })
    case "act":
      if (action === "double_click") return i18n.t("ui.computer.doubleClicked", { target })
      if (action === "right_click") return i18n.t("ui.computer.rightClicked", { target })
      if (action === "type") return i18n.t("ui.computer.typed", { target })
      if (action === "toggle") return i18n.t("ui.computer.toggled", { target })
      if (action === "select" || action === "add_to_selection") return i18n.t("ui.computer.selected", { target })
      if (action === "expand") return i18n.t("ui.computer.expanded", { target })
      if (action === "collapse") return i18n.t("ui.computer.collapsed", { target })
      if (action.startsWith("scroll")) return i18n.t("ui.computer.scrolled", { target })
      if (action === "focus") return i18n.t("ui.computer.focused", { target })
      if (action === "read") return i18n.t("ui.computer.readElement", { target })
      return i18n.t("ui.computer.clicked", { target })
    case "key": {
      const pressed = keys(text(args.keys))
      return pressed ? i18n.t("ui.computer.pressed", { keys: pressed }) : i18n.t("ui.computer.typedText")
    }
    case "launch":
      return i18n.t("ui.computer.opened", { app: clip(text(args.app).split(/[\\/]/).pop() ?? "") || app })
    case "focus":
      return i18n.t("ui.computer.switched", { app })
    case "ocr":
      return i18n.t("ui.computer.ocr", { app: app || i18n.t("ui.computer.readScreen") })
    case "screenshot":
      return i18n.t("ui.computer.screenshot")
    case "click_at":
      return i18n.t("ui.computer.clickedPoint")
    case "batch":
      return i18n.plural("ui.computer.batch", Array.isArray(args.steps) ? args.steps.length : 0)
    case "browser_open": {
      const url = text(args.url)
      if (["back", "forward", "reload"].includes(url)) return i18n.t("ui.computer.browserNavigated", { direction: url })
      return i18n.t("ui.computer.browserOpened", { site: host(url) })
    }
    case "browser_snapshot":
      return i18n.t("ui.computer.browserRead")
    case "browser_find":
      return i18n.t("ui.computer.browserFind", { text: clip(text(args.text)) })
    case "browser_act":
      return action === "type"
        ? i18n.t("ui.computer.browserTyped", { target })
        : i18n.t("ui.computer.browserClicked", { target })
    case "browser_key":
      return i18n.t("ui.computer.browserPressed", { keys: keys(text(args.keys)) ?? text(args.keys) })
    case "browser_downloads":
      return i18n.t("ui.computer.browserDownloads")
    case "browser_tabs":
    case "browser_tab":
      return i18n.t("ui.computer.browserTabs")
    case "browser_capture":
      return i18n.t("ui.computer.browserCapture")
    case "page_read":
      return i18n.t("ui.computer.previewRead")
    case "page_act":
      return action === "type" ? i18n.t("ui.computer.previewTyped") : i18n.t("ui.computer.previewClicked")
    case "page_errors":
      return i18n.t("ui.computer.previewErrors")
    case "page_screenshot":
      return i18n.t("ui.computer.previewScreenshot")
    case "preview":
      return i18n.t("ui.computer.previewStarted")
    case "preview_logs":
      return i18n.t("ui.computer.previewLogs")
    case "preview_stop":
      return i18n.t("ui.computer.previewStopped")
    default:
      return i18n.t("ui.tool.computer")
  }
}
