// Cryzo browser pane picker (injected into every top-level page of the pane). Select mode
// highlights the element under the pointer with its tag and size; a click picks it and a drag picks
// an area. Each pick gets a comment and is sent as a note to the Cryzo composer; in batch mode picks
// become numbered pins and are sent together. Edit hands an element to Cryzo's inspector, which
// previews style changes here and sends them with the note. Notes are text, as Claude's browser
// reads pages; marks and previewed edits clear once a note is sent. Messages leave the page through
// a navigation to a sentinel URL that the host reads and cancels; the page has no other channel to
// Cryzo.
;(() => {
  if (window.top !== window || window.__cryzoPicker) return
  // Errors the page reports since it loaded, for the agent's page_errors (read through DevTools).
  const errors = (window.__cryzoErrors = [])
  const record = (text) => {
    if (errors.length < 50) errors.push(String(text).slice(0, 1000))
  }
  const errorText = (value) => {
    if (value instanceof Error) return value.stack || value.message
    if (typeof value === "string") return value
    try {
      return JSON.stringify(value)
    } catch {
      return String(value)
    }
  }
  addEventListener("error", (event) => record(event.message + (event.filename ? ` (${event.filename}:${event.lineno})` : "")))
  addEventListener("unhandledrejection", (event) => record("Unhandled rejection: " + errorText(event.reason)))
  const consoleError = console.error
  console.error = (...args) => {
    record(args.map(errorText).join(" "))
    return consoleError.apply(console, args)
  }
  const SENTINEL = "https://cryzo-note.invalid/#"
  const SVG = "http://www.w3.org/2000/svg"
  // The properties the inspector shows and may change; anything else it asks for is ignored.
  const STYLES = [
    "color",
    "background-color",
    "opacity",
    "font-family",
    "font-size",
    "font-weight",
    "line-height",
    "letter-spacing",
    "text-align",
    "margin-top",
    "margin-right",
    "margin-bottom",
    "margin-left",
    "padding-top",
    "padding-right",
    "padding-bottom",
    "padding-left",
    "border-radius",
    "border-top-width",
    "border-top-color",
    "display",
    "width",
    "height",
    "flex-direction",
    "gap",
    "align-items",
    "justify-content",
  ]
  const BORDER = { "border-top-width": "border-width", "border-top-color": "border-color" }
  const state = { on: false, batch: false, pending: [], start: null, dragging: false, editing: false }
  let labels = { comment: "Comment for the agent", send: "Send", add: "Add", edit: "Edit" }
  let root, highlight, tip, area, box, pins, marks, selected

  function ui() {
    if (root) return
    const host = document.createElement("cryzo-picker")
    host.style.cssText = "position:fixed;inset:0;pointer-events:none;z-index:2147483647"
    root = host.attachShadow({ mode: "closed" })
    // Built with DOM calls, not innerHTML: pages with Trusted Types (YouTube, Google) refuse markup strings.
    const element = (tag, className, parent = root) => {
      const node = tag === "svg" ? document.createElementNS(SVG, tag) : document.createElement(tag)
      if (className) node.setAttribute("class", className)
      parent.append(node)
      return node
    }
    element("style").textContent = `
      .hl{position:fixed;border:2px solid #7c5cff;background:rgba(124,92,255,.12);border-radius:3px;pointer-events:none;display:none}
      .sel{position:fixed;border:2px solid #7c5cff;border-radius:3px;pointer-events:none;display:none}
      .tip{position:fixed;display:none;padding:2px 6px;border-radius:4px;background:#1d1b22;color:#fff;font:500 11px/16px ui-monospace,monospace;white-space:nowrap;pointer-events:none}
      .tip b{font-weight:500;color:#c4b5fd}
      .area{position:fixed;border:2px dashed #7c5cff;background:rgba(124,92,255,.08);pointer-events:none;display:none}
      .box{position:fixed;display:none;gap:6px;padding:6px;background:#1d1b22;border-radius:8px;box-shadow:0 6px 24px rgba(0,0,0,.3);pointer-events:auto;font:13px system-ui;color:#fff}
      .box input{width:240px;padding:6px 8px;border-radius:6px;border:1px solid #444;background:#2a2730;color:#fff;font:inherit}
      .box button{padding:6px 10px;border-radius:6px;border:0;background:#7c5cff;color:#fff;font:inherit;cursor:pointer}
      .box button.edit{background:#2a2730;border:1px solid #444}
      .marks{position:fixed;inset:0;width:100vw;height:100vh;pointer-events:none;overflow:visible}
      .marks rect{fill:rgba(124,92,255,.10);stroke:#7c5cff;stroke-width:2}
      .pin{position:fixed;width:20px;height:20px;margin:-10px 0 0 -10px;border-radius:50%;background:#7c5cff;color:#fff;font:600 11px/20px system-ui;text-align:center;pointer-events:none}
    `
    for (const name of ["marks:svg", "hl", "sel", "tip", "area", "pins"]) {
      const [className, tag = "div"] = name.split(":")
      element(tag, className)
    }
    const form = element("form", "box")
    element("input", "", form)
    element("button", "edit", form).type = "button"
    element("button", "", form).type = "submit"
    highlight = root.querySelector(".hl")
    tip = root.querySelector(".tip")
    area = root.querySelector(".area")
    pins = root.querySelector(".pins")
    marks = root.querySelector(".marks")
    box = root.querySelector(".box")
    box.addEventListener("submit", (event) => {
      event.preventDefault()
      commit(box.querySelector("input").value)
    })
    box.querySelector(".edit").addEventListener("click", edit)
    box.addEventListener("keydown", (event) => {
      if (event.key === "Escape") closeBox()
      event.stopPropagation()
    })
    document.documentElement.append(host)
  }

  function place(node, rect) {
    node.style.left = rect.left + "px"
    node.style.top = rect.top + "px"
    node.style.width = rect.width + "px"
    node.style.height = rect.height + "px"
  }

  function target(event) {
    const node = document.elementFromPoint(event.clientX, event.clientY)
    return node && node.nodeType === 1 && node.tagName !== "CRYZO-PICKER" ? node : null
  }

  function selector(node) {
    const parts = []
    for (let current = node; current && current.nodeType === 1 && parts.length < 8; current = current.parentElement) {
      if (current.id && document.querySelectorAll("#" + CSS.escape(current.id)).length === 1) {
        parts.unshift("#" + CSS.escape(current.id))
        break
      }
      const tag = current.tagName.toLowerCase()
      const parent = current.parentElement
      if (!parent || tag === "body" || tag === "html") {
        parts.unshift(tag)
        break
      }
      const same = Array.from(parent.children).filter((child) => child.tagName === current.tagName)
      parts.unshift(same.length > 1 ? `${tag}:nth-of-type(${same.indexOf(current) + 1})` : tag)
    }
    return parts.join(" > ")
  }

  const kept = ["name", "type", "role", "aria-label", "href", "src", "alt", "placeholder", "title", "data-testid", "value"]

  function describe(node) {
    const rect = node.getBoundingClientRect()
    const attributes = {}
    for (const name of kept) {
      const value = node.getAttribute(name)
      if (value !== null) attributes[name] = value.slice(0, 200)
    }
    return {
      tag: node.tagName.toLowerCase(),
      id: node.id || undefined,
      classes: Array.from(node.classList).slice(0, 20),
      attributes,
      selector: selector(node),
      text: (node.innerText || node.textContent || "").replace(/\s+/g, " ").trim().slice(0, 200),
      rect: { x: Math.round(rect.left), y: Math.round(rect.top), width: Math.round(rect.width), height: Math.round(rect.height) },
      component: component(node),
    }
  }

  // The framework component that rendered the element, from what dev builds leave on DOM nodes. The
  // page owns these objects, so a getter that throws only means there is no answer.
  function component(node) {
    try {
      for (let current = node, depth = 0; current && depth < 15; current = current.parentElement, depth++) {
        const fiber = Object.keys(current).find((key) => key.startsWith("__reactFiber$"))
        if (fiber) {
          const names = []
          for (let item = current[fiber]; item && names.length < 3; item = item.return) {
            const type = item.type
            const name = type && typeof type !== "string" ? type.displayName || type.name || type.render?.name : undefined
            if (typeof name === "string" && /^[A-Z]/.test(name) && !names.includes(name)) names.push(name)
          }
          return names.length ? { framework: "react", names } : undefined
        }
        const vue = current.__vueParentComponent
        if (vue) {
          const names = []
          for (let item = vue; item && names.length < 3; item = item.parent) {
            const name = item.type?.__name || item.type?.name
            if (typeof name === "string" && !names.includes(name)) names.push(name)
          }
          const file = vue.type?.__file
          return { framework: "vue", names, file: typeof file === "string" ? file : undefined }
        }
        const file = current.__svelte_meta?.loc?.file
        if (typeof file === "string") return { framework: "svelte", names: [], file }
      }
    } catch {
      return undefined
    }
  }

  function inside(rect) {
    const found = []
    for (const node of document.body.querySelectorAll("*")) {
      const box = node.getBoundingClientRect()
      if (box.width < 4 || box.height < 4) continue
      if (box.left >= rect.left && box.top >= rect.top && box.right <= rect.right && box.bottom <= rect.bottom) {
        if (!found.some((parent) => parent.contains(node))) found.push(node)
      }
      if (found.length >= 10) break
    }
    return found.map((node) => {
      const item = describe(node)
      return { tag: item.tag, selector: item.selector, text: item.text }
    })
  }

  let draft = null
  // The element the comment box is for; kept out of the note itself, which is sent as JSON.
  let draftNode = null

  function openBox(note, rect, node = null) {
    draft = note
    draftNode = node
    state.editing = true
    const input = box.querySelector("input")
    input.value = ""
    input.placeholder = labels.comment
    const editButton = box.querySelector(".edit")
    editButton.textContent = labels.edit
    editButton.style.display = node ? "" : "none"
    box.querySelector("[type=submit]").textContent = state.batch ? labels.add : labels.send
    box.style.display = "flex"
    box.style.left = Math.max(8, Math.min(rect.left, innerWidth - box.offsetWidth - 8)) + "px"
    box.style.top = Math.min(rect.bottom + 8, innerHeight - box.offsetHeight - 8) + "px"
    setTimeout(() => input.focus(), 0)
  }

  function closeBox() {
    box.style.display = "none"
    area.style.display = "none"
    state.editing = false
    draft = null
    draftNode = null
  }

  function outline(rect) {
    const node = document.createElementNS(SVG, "rect")
    for (const [name, value] of Object.entries(rect)) node.setAttribute(name, String(value))
    marks.append(node)
  }

  function commit(comment) {
    if (!draft) return
    const note = {
      ...draft,
      comment: comment.trim(),
      url: location.href,
      title: document.title.slice(0, 200),
      viewport: { width: innerWidth, height: innerHeight },
    }
    const rect = (draft.element || draft.area).rect
    outline(rect)
    closeBox()
    if (!state.batch) return send([note])
    state.pending.push(note)
    const pin = document.createElement("div")
    pin.className = "pin"
    pin.textContent = String(state.pending.length)
    pin.style.left = rect.x + "px"
    pin.style.top = rect.y + "px"
    pins.append(pin)
  }

  function send(notes) {
    if (!notes.length) return
    for (const node of [...marks.children, ...pins.children]) node.dataset.sent = ""
    for (const item of edits) item.sent = true
    location.href = SENTINEL + encodeURIComponent(JSON.stringify(notes))
    // The marks stay a moment so the user sees what was sent.
    setTimeout(finish, 600)
  }

  // Sent marks go, and sent edits are undone so the page shows the app as its code draws it again,
  // rather than hiding the agent's change behind leftover inline styles.
  function finish() {
    for (const node of root.querySelectorAll("[data-sent]")) node.remove()
    for (const item of edits.filter((item) => item.sent)) revert(item)
    edits = edits.filter((item) => !item.sent)
  }

  // Edit: the selected element, its original inline styles and text, and the changes previewed so far.
  let edits = []

  function edit() {
    const node = draftNode
    if (!node) return
    closeBox()
    highlight.style.display = "none"
    tip.style.display = "none"
    selected = { node, saved: new Map(), changes: new Map(), sent: false }
    edits.push(selected)
    showSelection()
    const computed = getComputedStyle(node)
    const styles = Object.fromEntries(STYLES.map((name) => [name, computed.getPropertyValue(name).slice(0, 200)]))
    const plain = Array.from(node.childNodes).every((child) => child.nodeType === 3)
    const text = plain ? (node.textContent || "").slice(0, 500) : undefined
    location.href = SENTINEL + encodeURIComponent(JSON.stringify({ selection: { element: describe(node), styles, text } }))
  }

  function showSelection() {
    const sel = root.querySelector(".sel")
    if (!selected || selected.sent) return (sel.style.display = "none")
    sel.style.display = "block"
    place(sel, selected.node.getBoundingClientRect())
  }

  function style(name, value) {
    if (!selected || typeof value !== "string") return
    const node = selected.node
    if (name === "text") {
      if (!selected.saved.has("text")) selected.saved.set("text", node.textContent)
      node.textContent = value
      selected.changes.set("text", { property: "text", from: selected.saved.get("text"), to: value })
      return showSelection()
    }
    if (!STYLES.includes(name)) return
    const property = BORDER[name] || name
    if (!selected.saved.has(property)) {
      selected.saved.set(property, [node.style.getPropertyValue(property), node.style.getPropertyPriority(property)])
      const before = getComputedStyle(node).getPropertyValue(name)
      selected.changes.set(name, { property: name, from: before, to: value })
    }
    node.style.setProperty(property, value, "important")
    if (property === "border-width" && getComputedStyle(node).borderTopStyle === "none") {
      if (!selected.saved.has("border-style")) selected.saved.set("border-style", [node.style.getPropertyValue("border-style"), node.style.getPropertyPriority("border-style")])
      node.style.setProperty("border-style", "solid", "important")
    }
    selected.changes.get(name).to = value
    showSelection()
  }

  function revert(item) {
    for (const [property, saved] of item.saved) {
      if (property === "text") item.node.textContent = saved
      else if (saved[0]) item.node.style.setProperty(property, saved[0], saved[1])
      else item.node.style.removeProperty(property)
    }
    item.saved.clear()
    item.changes.clear()
  }

  function onMove(event) {
    if (!state.on || state.editing) return
    if (state.start) {
      const dx = event.clientX - state.start.x
      const dy = event.clientY - state.start.y
      if (!state.dragging && Math.hypot(dx, dy) > 6) state.dragging = true
      if (state.dragging) {
        highlight.style.display = "none"
        tip.style.display = "none"
        area.style.display = "block"
        place(area, {
          left: Math.min(event.clientX, state.start.x),
          top: Math.min(event.clientY, state.start.y),
          width: Math.abs(dx),
          height: Math.abs(dy),
        })
        return
      }
    }
    const node = target(event)
    if (!node) {
      highlight.style.display = "none"
      tip.style.display = "none"
      return
    }
    const rect = node.getBoundingClientRect()
    highlight.style.display = "block"
    place(highlight, rect)
    // Like a DevTools hover: the element's tag, id or first class, and size.
    const name = node.tagName.toLowerCase() + (node.id ? "#" + node.id : node.classList[0] ? "." + node.classList[0] : "")
    tip.replaceChildren(name.slice(0, 60) + " ")
    const size = document.createElement("b")
    size.textContent = `${Math.round(rect.width)}×${Math.round(rect.height)}`
    tip.append(size)
    tip.style.display = "block"
    tip.style.left = Math.max(4, Math.min(rect.left, innerWidth - tip.offsetWidth - 4)) + "px"
    tip.style.top = (rect.top >= 24 ? rect.top - 22 : Math.min(rect.bottom + 4, innerHeight - 20)) + "px"
  }

  function onDown(event) {
    if (!state.on || state.editing || event.button !== 0) return
    event.preventDefault()
    event.stopPropagation()
    state.start = { x: event.clientX, y: event.clientY }
    state.dragging = false
  }

  function onUp(event) {
    if (!state.on || state.editing || !state.start) return
    event.preventDefault()
    event.stopPropagation()
    const start = state.start
    state.start = null
    tip.style.display = "none"
    if (state.dragging) {
      state.dragging = false
      const rect = {
        left: Math.min(event.clientX, start.x),
        top: Math.min(event.clientY, start.y),
        right: Math.max(event.clientX, start.x),
        bottom: Math.max(event.clientY, start.y),
      }
      const note = {
        kind: "area",
        area: {
          rect: { x: Math.round(rect.left), y: Math.round(rect.top), width: Math.round(rect.right - rect.left), height: Math.round(rect.bottom - rect.top) },
          elements: inside(rect),
        },
      }
      return openBox(note, rect)
    }
    const node = target(event)
    if (!node) return
    highlight.style.display = "none"
    openBox({ kind: "element", element: describe(node) }, node.getBoundingClientRect(), node)
  }

  // Clicks are swallowed in select mode so links and buttons do not act while picking.
  function onClick(event) {
    if (!state.on || state.editing) return
    event.preventDefault()
    event.stopPropagation()
  }

  for (const [type, handler] of [
    ["mousemove", onMove],
    ["mousedown", onDown],
    ["mouseup", onUp],
    ["click", onClick],
  ])
    window.addEventListener(type, handler, true)
  addEventListener("scroll", showSelection, true)
  addEventListener("resize", showSelection)

  window.__cryzoPicker = {
    set(on, batch, text) {
      ui()
      state.on = on
      state.batch = batch
      if (text && typeof text === "object") labels = { ...labels, ...text }
      highlight.style.display = "none"
      tip.style.display = "none"
      document.documentElement.style.cursor = on ? "crosshair" : ""
      if (!on) closeBox()
    },
    flush() {
      send(state.pending.splice(0))
    },
    style,
    reset() {
      if (selected) revert(selected)
      showSelection()
    },
    cancelEdit() {
      if (!selected) return
      revert(selected)
      edits = edits.filter((item) => item !== selected)
      selected = null
      showSelection()
    },
    // Sends the edited element as a note: its details, the requested changes and the comment.
    sendEdit(comment) {
      if (!selected) return
      const item = selected
      selected = null
      showSelection()
      draft = { kind: "element", element: describe(item.node), changes: Array.from(item.changes.values()) }
      commit(typeof comment === "string" ? comment : "")
    },
  }
})()
