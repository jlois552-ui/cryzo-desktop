// Cryzo remote: the regular Cryzo web app, running against the user's desktop engine through the
// relay. Pairing happens once with a code from Settings > Remote control on the computer.
// The app's fonts and color tokens, loaded first so the pairing screen looks like Cryzo too.
import "@/index.css"
import en from "@/runtime/i18n/en"
import { Relay } from "./relay"

type Credential = { host: string; client: string; secret: string }

const gateway = (import.meta.env.VITE_CRYZO_GATEWAY_URL as string | undefined)?.replace(/\/+$/, "")
const storageKey = "cryzo.remote"

const credential = await paired()
const relay = new Relay()
const banner = statusBanner()
await connect(relay, credential)
install(relay)
// The real app, unchanged. Its server is this page's origin; the shims above carry `/api/` to the computer.
await import("@/entry")

/** The saved device credential, or a new one from the pairing screen. */
async function paired(): Promise<Credential> {
  const code = new URLSearchParams(location.search).get("pair")
  const saved = load()
  if (saved && !code) return saved
  const claimed = await pairingScreen(code ?? "")
  history.replaceState(null, "", location.pathname)
  return claimed
}

/**
 * Keeps one relay connection open for as long as the page is open. It redials with a fresh ticket
 * after any drop, and pings the relay every 20 s, and at once whenever the page comes back to the
 * front, because iOS freezes background tabs and their sockets die without a close event.
 */
async function connect(relay: Relay, credential: Credential) {
  let socket: WebSocket | undefined
  let dialing = false
  let delay = 1000
  let heard = 0
  let retry: ReturnType<typeof setTimeout> | undefined
  // Whether the app has started, and whether the computer was online when it did.
  let started = false
  let startedOnline = false

  const dial = async () => {
    if (dialing || socket) return
    dialing = true
    clearTimeout(retry)
    banner(en["remote.connecting"])
    try {
      socket = await open()
      delay = 1000
    } catch {
      retry = setTimeout(() => void dial(), delay)
      delay = Math.min(delay * 2, 30_000)
    } finally {
      dialing = false
    }
  }

  const open = async () => {
    const response = await gatewayPost(
      "remote/ticket",
      { role: "client", id: credential.client, secret: credential.secret },
      AbortSignal.timeout(10_000),
    )
    if (response.status === 401) {
      // This device was removed on the computer; pair again.
      localStorage.removeItem(storageKey)
      location.reload()
      return new Promise<never>(() => undefined)
    }
    const data = (await response.json()) as { data?: { ticket: string; relay: string } }
    if (!data.data) throw new Error("no ticket")
    const url = new URL(data.data.relay)
    url.searchParams.set("ticket", data.data.ticket)
    const next = new WebSocket(url)
    await new Promise<void>((resolve, reject) => {
      const timeout = setTimeout(() => {
        next.close()
        reject(new Error("relay timed out"))
      }, 10_000)
      next.onopen = () => {
        clearTimeout(timeout)
        resolve()
      }
      next.onerror = () => {
        clearTimeout(timeout)
        reject(new Error("relay unreachable"))
      }
    })
    heard = Date.now()
    relay.attach(next)
    next.onmessage = (event) => {
      heard = Date.now()
      if (event.data !== "pong") relay.deliver(event.data)
    }
    next.onclose = () => drop(next)
    return next
  }

  // Forgets a socket at once, without waiting for a dead one's close handshake, and redials.
  const drop = (dead: WebSocket) => {
    if (socket !== dead) return
    socket = undefined
    dead.onclose = null
    dead.onmessage = null
    dead.close()
    relay.detach()
    void dial()
  }

  // The relay answers "ping" with "pong"; silence past `patience` means the socket is gone.
  const probe = (patience: number) => {
    const current = socket
    if (!current) {
      delay = 1000
      return void dial()
    }
    const sent = Date.now()
    current.send("ping")
    setTimeout(() => {
      if (heard < sent) drop(current)
    }, patience)
  }
  setInterval(() => probe(10_000), 20_000)
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "visible") probe(4000)
  })
  addEventListener("pageshow", () => probe(4000))
  addEventListener("online", () => probe(4000))

  // The app starts once the computer answers, or after 5 s either way so the page never hangs. If it
  // had to start while the computer was offline, the page reloads when it comes back, so the app
  // loads its data from a working connection.
  const ready = new Promise<void>((resolve) => {
    relay.onstatus = (online) => {
      banner(online ? "" : en["remote.offline"])
      if (!online) return
      if (started && !startedOnline) location.reload()
      resolve()
    }
    setTimeout(resolve, 5000)
  })
  await dial()
  await ready
  started = true
  startedOnline = relay.online
}

/** Routes this origin's `/api/` requests and terminal sockets through the relay. */
function install(relay: Relay) {
  const browserFetch = window.fetch.bind(window)
  const ours = (url: URL) => url.host === location.host && url.pathname.startsWith("/api/")
  window.fetch = async (input, init) => {
    const request = new Request(input, init)
    return ours(new URL(request.url)) ? relay.fetch(request) : browserFetch(request)
  }
  window.WebSocket = new Proxy(window.WebSocket, {
    construct(target, args) {
      const url = new URL(String(args[0]), location.href)
      if (!ours(url)) return new target(...(args as ConstructorParameters<typeof WebSocket>))
      return relay.open(url.pathname + url.search)
    },
  })
}

function load(): Credential | undefined {
  try {
    const value = JSON.parse(localStorage.getItem(storageKey) ?? "null") as Credential | null
    return value?.host && value.client && value.secret ? value : undefined
  } catch {
    return undefined
  }
}

async function gatewayPost(path: string, body: unknown, signal?: AbortSignal) {
  if (!gateway) throw new Error("VITE_CRYZO_GATEWAY_URL is not set for this build.")
  return fetch(`${gateway}/${path}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
    signal,
  })
}

function pairingScreen(code: string) {
  const root = document.getElementById("root")!
  root.innerHTML = ""
  // Styled on its own page element: the app later renders into #root, which must keep its own layout.
  const page = element("div", `flex:1;display:flex;align-items:center;background:${token("background-bg-base", "#fff")}`)
  root.append(page)
  const form = element(
    "form",
    `display:flex;flex-direction:column;gap:16px;box-sizing:border-box;width:100%;max-width:400px;margin:0 auto;padding:24px;font-family:var(--font-family-sans);color:${token("text-text-base", "#111")}`,
  )
  const title = element("h1", "font-size:22px;line-height:28px;font-weight:600;margin:0")
  title.textContent = en["remote.pair.title"]
  const help = element("p", `margin:0;font-size:15px;line-height:22px;color:${token("text-text-muted", "#666")}`)
  help.textContent = en["remote.pair.description"]
  const codeInput = field(en["remote.pair.code"], code, "K7QF-2M9X")
  codeInput.input.autocapitalize = "characters"
  const nameInput = field(en["remote.pair.name"], deviceName(), "")
  const error = element("p", "margin:0;font-size:14px;color:#d33")
  error.setAttribute("role", "alert")
  const submit = element(
    "button",
    `height:48px;border:0;border-radius:12px;font:inherit;font-size:16px;font-weight:600;cursor:pointer;background:${token("text-text-base", "#111")};color:${token("background-bg-base", "#fff")}`,
  )
  submit.textContent = en["remote.pair.submit"]
  form.append(title, help, codeInput.label, nameInput.label, error, submit)
  page.append(form)
  // Pairing ends in a reload, so this promise only settles by leaving the page.
  return new Promise<Credential>(() => {
    form.onsubmit = (event) => {
      event.preventDefault()
      submit.disabled = true
      submit.textContent = en["remote.pair.working"]
      error.textContent = ""
      void gatewayPost("remote/claim", { code: codeInput.input.value, name: nameInput.input.value || deviceName() })
        .then(async (response) => {
          const body = (await response.json()) as { data?: Credential; error?: string }
          if (!body.data) throw new Error(body.error ?? en["remote.pair.failed"])
          localStorage.setItem(storageKey, JSON.stringify(body.data))
          // Start the app from a fresh page, exactly as a later visit does, rather than inside the
          // document the pairing screen laid out.
          location.replace(location.pathname)
        })
        .catch((failure: unknown) => {
          error.textContent = failure instanceof Error ? failure.message : en["remote.pair.failed"]
          submit.disabled = false
          submit.textContent = en["remote.pair.submit"]
        })
    }
  })
}

function field(text: string, value: string, placeholder: string) {
  const label = element("label", `display:flex;flex-direction:column;gap:6px;font-size:13px;color:${token("text-text-muted", "#666")}`)
  label.textContent = text
  // 17px keeps phones from zooming into the field.
  const input = element(
    "input",
    `box-sizing:border-box;width:100%;height:48px;padding:0 14px;border-radius:12px;border:1px solid ${token("border-border-base", "#ccc")};background:${token("background-bg-base", "#fff")};color:${token("text-text-base", "#111")};font:inherit;font-size:17px`,
  )
  input.value = value
  input.placeholder = placeholder
  label.append(input)
  return { label, input }
}

/** A Cryzo color token, with a plain color while the theme has not applied its values yet. */
function token(name: string, fallback: string) {
  return `var(--v2-${name}, ${fallback})`
}

function element<Tag extends keyof HTMLElementTagNameMap>(tag: Tag, style: string) {
  const node = document.createElement(tag)
  node.style.cssText = style
  return node
}

function deviceName() {
  const agent = navigator.userAgent
  if (/iPhone/.test(agent)) return "iPhone"
  if (/iPad/.test(agent)) return "iPad"
  if (/Android/.test(agent)) return "Android phone"
  if (/Mac/.test(agent)) return "Mac browser"
  if (/Windows/.test(agent)) return "Windows browser"
  return "Browser"
}

/** A thin strip at the top while the computer is unreachable; empty text hides it. */
function statusBanner() {
  const node = element(
    "div",
    "position:fixed;top:0;left:0;right:0;z-index:2147483647;padding:6px 12px;font-size:13px;text-align:center;background:#222;color:#fff;display:none",
  )
  node.setAttribute("role", "status")
  document.body.append(node)
  return (text: string) => {
    node.textContent = text
    node.style.display = text ? "block" : "none"
  }
}
