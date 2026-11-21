// The DevTools Protocol calls the agent's page tools make (see native/engine/src/preview.rs),
// answered inside the page for WebKit, which has no DevTools Protocol to call: WKWebView on macOS
// and WebKitGTK on Linux. WebView2 on Windows answers the real protocol instead.
//
// The accessibility tree is built from the DOM with the usual ARIA rules, in the shape Chrome's
// Accessibility.getFullAXTree returns, so the shared formatter prints the same lines everywhere.
// Input is dispatched as DOM events: pages see ordinary clicks, input and key events.
async function cryzoCdp(method, params) {
  const state = (window.__cryzoCdp ||= { ids: new WeakMap(), nodes: new Map(), next: 1 })
  const idOf = (node) => {
    let id = state.ids.get(node)
    if (!id) {
      id = state.next++
      state.ids.set(node, id)
    }
    state.nodes.set(id, new WeakRef(node))
    return id
  }
  const nodeOf = (id) => {
    const node = state.nodes.get(Number(String(id).replace(/^n/, "")))?.deref()
    if (!node || !node.isConnected) throw new Error("the element isn't on the page any more")
    return node
  }

  switch (method) {
    case "Runtime.evaluate": {
      try {
        const value = await (0, eval)(params.expression)
        return { result: { type: typeof value, value: JSON.parse(JSON.stringify(value ?? null)) } }
      } catch (error) {
        return { exceptionDetails: { text: String(error), exception: { description: String(error?.stack || error) } } }
      }
    }
    case "DOM.enable":
      return {}
    case "DOM.resolveNode":
      return { object: { objectId: `n${idOf(nodeOf(params.backendNodeId))}` } }
    case "Runtime.callFunctionOn": {
      const target = nodeOf(params.objectId)
      const fn = (0, eval)(`(${params.functionDeclaration})`)
      const value = await fn.apply(target, (params.arguments || []).map((argument) => argument.value))
      return { result: { value: JSON.parse(JSON.stringify(value ?? null)) } }
    }
    case "DOM.scrollIntoViewIfNeeded":
      nodeOf(params.backendNodeId).scrollIntoView({ block: "center", inline: "nearest" })
      return {}
    case "DOM.getBoxModel": {
      const box = nodeOf(params.backendNodeId).getBoundingClientRect()
      if (!box.width && !box.height) throw new Error("the element has no box")
      const quad = [box.left, box.top, box.right, box.top, box.right, box.bottom, box.left, box.bottom]
      return { model: { content: quad, border: quad, width: box.width, height: box.height } }
    }
    case "DOM.focus":
      nodeOf(params.backendNodeId).focus()
      return {}
    case "Input.dispatchMouseEvent": {
      const target = document.elementFromPoint(params.x, params.y)
      if (!target) throw new Error("nothing on the page at that point")
      const init = { bubbles: true, cancelable: true, composed: true, clientX: params.x, clientY: params.y, button: 0, detail: params.clickCount || 1, view: window }
      if (params.type === "mouseMoved") {
        target.dispatchEvent(new PointerEvent("pointermove", init))
        target.dispatchEvent(new MouseEvent("mousemove", init))
      }
      if (params.type === "mousePressed") {
        target.dispatchEvent(new PointerEvent("pointerdown", init))
        target.dispatchEvent(new MouseEvent("mousedown", init))
        if (typeof target.focus === "function") target.focus()
      }
      if (params.type === "mouseReleased") {
        target.dispatchEvent(new PointerEvent("pointerup", init))
        target.dispatchEvent(new MouseEvent("mouseup", init))
        // A real click: links follow, buttons submit, checkboxes toggle.
        target.click()
      }
      return {}
    }
    case "Input.insertText": {
      const field = document.activeElement
      // execCommand keeps the page's undo stack and fires the input events frameworks listen to.
      if (!document.execCommand("insertText", false, params.text) && field && "value" in field) {
        field.value += params.text
        field.dispatchEvent(new InputEvent("input", { bubbles: true, data: params.text, inputType: "insertText" }))
      }
      return {}
    }
    case "Input.dispatchKeyEvent": {
      const target = document.activeElement || document.body
      const type = params.type === "keyUp" ? "keyup" : "keydown"
      const event = new KeyboardEvent(type, { key: params.key, code: params.code, bubbles: true, cancelable: true, composed: true })
      const allowed = target.dispatchEvent(event)
      // Synthetic Enter doesn't submit a form by itself, as a real one does.
      if (allowed && type === "keydown" && params.key === "Enter" && target.form && target.tagName === "INPUT") {
        target.form.requestSubmit()
      }
      return {}
    }
    case "Accessibility.getFullAXTree":
      return { nodes: tree(idOf) }
    case "Page.captureScreenshot":
      throw new Error("page screenshots aren't available in this browser pane yet; read the page with page_read")
    default:
      throw new Error(`${method} isn't available in this browser pane`)
  }
}

// The page as Chrome-style accessibility nodes: role, name, states and children.
function tree(idOf) {
  const nodes = []
  const hidden = (element) => {
    if (element.hidden || element.getAttribute("aria-hidden") === "true") return true
    const style = getComputedStyle(element)
    return style.display === "none" || style.visibility === "hidden"
  }
  const text = (node) => (node.textContent || "").replace(/\s+/g, " ").trim()
  const labelled = (element) => {
    const ids = element.getAttribute("aria-labelledby")
    if (ids) return ids.split(/\s+/).map((id) => text(document.getElementById(id) || document.createElement("i"))).join(" ").trim()
    const label = element.getAttribute("aria-label")
    if (label) return label.trim()
    if (element.labels?.length) return [...element.labels].map(text).join(" ").trim()
    return ""
  }
  const roleOf = (element) => {
    const explicit = element.getAttribute("role")
    if (explicit) return explicit.split(/\s+/)[0]
    const tag = element.tagName.toLowerCase()
    const type = (element.getAttribute("type") || "text").toLowerCase()
    if (tag === "a" && element.hasAttribute("href")) return "link"
    if (tag === "button" || (tag === "input" && ["button", "submit", "reset", "image"].includes(type))) return "button"
    if (tag === "input") {
      if (type === "checkbox") return "checkbox"
      if (type === "radio") return "radio"
      if (type === "range") return "slider"
      if (type === "search") return "searchbox"
      if (type === "hidden") return "none"
      return "textbox"
    }
    if (tag === "textarea") return "textbox"
    if (tag === "select") return element.multiple ? "listbox" : "combobox"
    if (tag === "option") return "option"
    if (tag === "img") return element.getAttribute("alt") === "" ? "presentation" : "image"
    if (/^h[1-6]$/.test(tag)) return "heading"
    if (tag === "nav") return "navigation"
    if (tag === "main") return "main"
    if (tag === "header") return "banner"
    if (tag === "footer") return "contentinfo"
    if (tag === "form") return "form"
    if (tag === "dialog") return "dialog"
    if (tag === "ul" || tag === "ol") return "list"
    if (tag === "li") return "listitem"
    if (tag === "table") return "table"
    if (tag === "tr") return "row"
    if (tag === "td") return "cell"
    if (tag === "th") return "columnheader"
    if (tag === "label") return "LabelText"
    if (tag === "p") return "paragraph"
    if (element.isContentEditable && element === element.closest("[contenteditable]")) return "textbox"
    return "generic"
  }
  const nameOf = (element, role) => {
    const label = labelled(element)
    if (label) return label
    if (role === "image") return element.getAttribute("alt") || element.getAttribute("title") || ""
    if (["button", "link", "heading", "option", "tab", "menuitem", "listitem", "cell", "columnheader", "checkbox", "radio", "LabelText", "paragraph"].includes(role)) {
      if (element.tagName === "INPUT") return element.value || element.getAttribute("title") || ""
      return text(element).slice(0, 200)
    }
    return element.getAttribute("title") || element.getAttribute("placeholder") || ""
  }
  const property = (name, value) => ({ name, value: { type: typeof value === "boolean" ? "boolean" : "string", value } })
  const visit = (element) => {
    const id = idOf(element)
    const role = roleOf(element)
    const ignored = role === "none" || role === "presentation"
    const properties = []
    if (element.disabled || element.getAttribute("aria-disabled") === "true") properties.push(property("disabled", true))
    if (document.activeElement === element) properties.push(property("focused", true))
    if ("checked" in element && ["checkbox", "radio"].includes(role)) properties.push(property("checked", element.checked ? "true" : "false"))
    else if (element.hasAttribute("aria-checked")) properties.push(property("checked", element.getAttribute("aria-checked")))
    if (element.hasAttribute("aria-expanded")) properties.push(property("expanded", element.getAttribute("aria-expanded") === "true"))
    if (element.hasAttribute("aria-selected") || element.selected) properties.push(property("selected", element.selected || element.getAttribute("aria-selected") === "true"))
    if (element.required || element.getAttribute("aria-required") === "true") properties.push(property("required", true))
    const node = { nodeId: String(id), backendDOMNodeId: id, ignored, role: { type: "role", value: role }, name: { type: "computedString", value: nameOf(element, role) }, properties, childIds: [] }
    if ("value" in element && ["textbox", "searchbox", "combobox", "slider"].includes(role) && element.type !== "password") {
      node.value = { type: "string", value: String(element.value ?? "") }
    }
    nodes.push(node)
    for (const child of element.children) {
      if (hidden(child)) continue
      node.childIds.push(visit(child))
    }
    // Text directly inside an element with children, such as "Hello <b>world</b>".
    for (const child of element.childNodes) {
      if (child.nodeType !== Node.TEXT_NODE || !child.textContent.trim() || element.children.length === 0) continue
      const textId = idOf(child)
      nodes.push({ nodeId: String(textId), backendDOMNodeId: textId, ignored: false, role: { type: "role", value: "StaticText" }, name: { type: "computedString", value: text(child) }, properties: [], childIds: [] })
      node.childIds.push(String(textId))
    }
    return String(id)
  }
  const rootId = idOf(document)
  const root = { nodeId: String(rootId), backendDOMNodeId: rootId, ignored: false, role: { type: "role", value: "RootWebArea" }, name: { type: "computedString", value: document.title }, properties: [], childIds: [] }
  nodes.push(root)
  if (document.body) root.childIds.push(visit(document.body))
  return nodes
}
