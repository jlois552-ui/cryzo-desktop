import { invoke } from "@tauri-apps/api/core"

// OpenCode reserves 138 physical pixels for the Windows host's caption buttons.
// Supply that host surface without changing the original application titlebar.
export function installNativeChrome() {
  if (!navigator.userAgent.includes("Windows")) return () => {}
  const controls = document.createElement("div")
  controls.id = "cryzo-window-controls"
  const style = document.createElement("style")
  style.textContent = `
    #cryzo-window-controls { position:fixed;right:0;top:0;z-index:2147483647;display:flex;height:calc(44px / var(--caption-zoom,1));color:var(--v2-icon-icon-base,currentColor);app-region:no-drag }
    #cryzo-window-controls[hidden] { display:none }
    #cryzo-window-controls button { appearance:none;border:0;border-radius:0;padding:0;display:grid;place-items:center;width:calc(46px / var(--caption-zoom,1));height:100%;color:inherit;background:transparent }
    #cryzo-window-controls button:hover { background:color-mix(in srgb,currentColor 12%,transparent) }
    #cryzo-window-controls button:last-child:hover { background:#c42b1c;color:white }
    #cryzo-window-controls svg { width:calc(10px / var(--caption-zoom,1));height:calc(10px / var(--caption-zoom,1));fill:none;stroke:currentColor;stroke-width:1 }
    [data-tauri-drag-region] { app-region:drag }
  `
  document.head.append(style)
  for (const [action, label, path] of [
    ["window.minimize", "Minimize", "M0 5h10"],
    ["window.toggleMaximize", "Maximize", "M.5.5h9v9h-9z"],
    ["window.close", "Close", "M.5.5l9 9m0-9l-9 9"],
  ]) {
    const button = document.createElement("button")
    button.type = "button"
    button.setAttribute("aria-label", label)
    const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg")
    svg.setAttribute("viewBox", "0 0 10 10")
    svg.setAttribute("aria-hidden", "true")
    const drawing = document.createElementNS("http://www.w3.org/2000/svg", "path")
    drawing.setAttribute("d", path)
    svg.append(drawing)
    button.append(svg)
    button.addEventListener("click", () => void invoke("desktop_call", { tag: "MenuRunAction", payload: { action } }))
    controls.append(button)
  }
  document.body.append(controls)
  return (event: { _tag: string } & Record<string, unknown>) => {
    if (event._tag === "WindowZoomChanged" && typeof event.factor === "number")
      controls.style.setProperty("--caption-zoom", String(Math.max(event.factor, 1)))
    if (event._tag === "WindowFullscreenChanged") controls.hidden = event.fullscreen === true
    if (event._tag === "WindowCaptionChanged") {
      controls.hidden = event.fullscreen === true
      controls.children[1]?.setAttribute("aria-label", event.maximized ? "Restore" : "Maximize")
    }
  }
}
