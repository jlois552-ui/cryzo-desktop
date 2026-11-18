// Cryzo browser pane picker (injected into every top-level page of the pane). Annotate mode
// highlights the element under the pointer; a click picks it, a drag picks an area. Each pick gets
// a comment and is sent as a note to the Cryzo composer. In batch mode picks become numbered pins
// and are sent together. Notes leave the page through a navigation to a sentinel URL that the
// host reads and cancels; the page has no other channel to Cryzo.
;(() => {
  if (window.top !== window || window.__cryzoPicker) return
  const SENTINEL = "https://cryzo-note.invalid/#"
  const state = { on: false, batch: false, pending: [], start: null, dragging: false, editing: false }
  let root, highlight, area, box, pins

  function ui() {
    if (root) return
    const host = document.createElement("cryzo-picker")
    host.style.cssText = "position:fixed;inset:0;pointer-events:none;z-index:2147483647"
    root = host.attachShadow({ mode: "closed" })
    root.innerHTML = `<style>
      .hl{position:fixed;border:2px solid #7c5cff;background:rgba(124,92,255,.12);border-radius:3px;pointer-events:none;display:none}
      .area{position:fixed;border:2px dashed #7c5cff;background:rgba(124,92,255,.08);pointer-events:none;display:none}
      .box{position:fixed;display:none;gap:6px;padding:6px;background:#1d1b22;border-radius:8px;box-shadow:0 6px 24px rgba(0,0,0,.3);pointer-events:auto;font:13px system-ui;color:#fff}
      .box input{width:240px;padding:6px 8px;border-radius:6px;border:1px solid #444;background:#2a2730;color:#fff;font:inherit}
      .box button{padding:6px 10px;border-radius:6px;border:0;background:#7c5cff;color:#fff;font:inherit;cursor:pointer}
      .pin{position:fixed;width:20px;height:20px;margin:-10px 0 0 -10px;border-radius:50%;background:#7c5cff;color:#fff;font:600 11px/20px system-ui;text-align:center;pointer-events:none}
    </style><div class="hl"></div><div class="area"></div><div class="pins"></div>
    <form class="box"><input placeholder="Comment for the agent" /><button type="submit">Send</button></form>`
    highlight = root.querySelector(".hl")
    area = root.querySelector(".area")
    pins = root.querySelector(".pins")
    box = root.querySelector(".box")
    box.addEventListener("submit", (event) => {
      event.preventDefault()
      commit(box.querySelector("input").value)
    })
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

  function openBox(note, rect) {
    draft = note
    state.editing = true
    box.style.display = "flex"
    box.style.left = Math.max(8, Math.min(rect.left, innerWidth - 320)) + "px"
    box.style.top = Math.min(rect.bottom + 8, innerHeight - 48) + "px"
    const input = box.querySelector("input")
    input.value = ""
    box.querySelector("button").textContent = state.batch ? "Add" : "Send"
    setTimeout(() => input.focus(), 0)
  }

  function closeBox() {
    box.style.display = "none"
    area.style.display = "none"
    state.editing = false
    draft = null
  }

  function commit(comment) {
    if (!draft) return
    const note = { ...draft, comment: comment.trim(), url: location.href, title: document.title.slice(0, 200) }
    const rect = draft.element ? draft.element.rect : draft.area.rect
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
    location.href = SENTINEL + encodeURIComponent(JSON.stringify(notes))
  }

  function onMove(event) {
    if (!state.on || state.editing) return
    if (state.start) {
      const dx = event.clientX - state.start.x
      const dy = event.clientY - state.start.y
      if (!state.dragging && Math.hypot(dx, dy) > 6) state.dragging = true
      if (state.dragging) {
        highlight.style.display = "none"
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
    if (!node) return (highlight.style.display = "none")
    highlight.style.display = "block"
    place(highlight, node.getBoundingClientRect())
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
    openBox({ kind: "element", element: describe(node) }, node.getBoundingClientRect())
  }

  // Clicks are swallowed in annotate mode so links and buttons do not act while picking.
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

  window.__cryzoPicker = {
    set(on, batch) {
      ui()
      state.on = on
      state.batch = batch
      document.documentElement.style.cursor = on ? "crosshair" : ""
      if (!on) {
        highlight.style.display = "none"
        closeBox()
      }
    },
    flush() {
      const notes = state.pending.splice(0)
      if (pins) pins.innerHTML = ""
      send(notes)
    },
  }
})()
