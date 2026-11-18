// Cryzo remote: the regular Cryzo web app, running against the user's desktop engine through the
// relay. Pairing happens once with a code from Settings > Remote control on the computer.
import en from "@/runtime/i18n/en"
import { Relay } from "./relay"

type Credential = { host: string; client: string; secret: string }

const gateway = (import.meta.env.VITE_CRYZO_GATEWAY_URL as string | undefined)?.replace(/\/+$/, "")
const storageKey = "cryzo.remote"

const credential = await paired()
const relay = new Relay()
const banner = statusBanner()
relay.onstatus = (online) => banner(online ? "" : en["remote.offline"])
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

/** Keeps one relay connection open, reconnecting with a fresh ticket after any drop. */
async function connect(relay: Relay, credential: Credential) {
  let delay = 1000
  const open = async (): Promise<void> => {
    const response = await gatewayPost("remote/ticket", { role: "client", id: credential.client, secret: credential.secret })
    if (response.status === 401) {
      // This device was removed on the computer; pair again.
      localStorage.removeItem(storageKey)
      location.reload()
      return new Promise(() => undefined)
    }
    const data = (await response.json()) as { data?: { ticket: string; relay: string } }
    if (!data.data) throw new Error("no ticket")
    const url = new URL(data.data.relay)
    url.searchParams.set("ticket", data.data.ticket)
    const socket = new WebSocket(url)
    await new Promise<void>((resolve, reject) => {
      socket.onopen = () => resolve()
      socket.onerror = () => reject(new Error("relay unreachable"))
    })
    relay.attach(socket)
    socket.addEventListener("message", (event) => relay.deliver(event.data))
    delay = 1000
    socket.addEventListener("close", () => {
      relay.detach()
      setTimeout(() => void retry(), delay)
    })
  }
  const retry = async (): Promise<void> => {
    try {
      await open()
    } catch {
      banner(en["remote.connecting"])
      delay = Math.min(delay * 2, 30_000)
      setTimeout(() => void retry(), delay)
    }
  }
  banner(en["remote.connecting"])
  await retry()
  // Wait briefly for the computer to answer, so the app's first requests are not lost.
  await new Promise<void>((resolve) => {
    if (relay.online) return resolve()
    const previous = relay.onstatus
    relay.onstatus = (online) => {
      previous?.(online)
      if (online) resolve()
    }
    setTimeout(resolve, 5000)
  })
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

async function gatewayPost(path: string, body: unknown) {
  if (!gateway) throw new Error("VITE_CRYZO_GATEWAY_URL is not set for this build.")
  return fetch(`${gateway}/${path}`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) })
}

function pairingScreen(code: string) {
  const root = document.getElementById("root")!
  root.innerHTML = ""
  const form = element("form", "display:flex;flex-direction:column;gap:12px;max-width:360px;margin:auto;padding:24px;width:100%")
  const title = element("h1", "font-size:20px;margin:0")
  title.textContent = en["remote.pair.title"]
  const help = element("p", "margin:0;opacity:.7")
  help.textContent = en["remote.pair.description"]
  const codeInput = field(en["remote.pair.code"], code, "K7QF-2M9X")
  codeInput.input.autocapitalize = "characters"
  const nameInput = field(en["remote.pair.name"], deviceName(), "")
  const error = element("p", "margin:0;color:#d33")
  error.setAttribute("role", "alert")
  const submit = element("button", "padding:10px;font-size:16px")
  submit.textContent = en["remote.pair.submit"]
  form.append(title, help, codeInput.label, nameInput.label, error, submit)
  root.append(form)
  return new Promise<Credential>((resolve) => {
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
          root.innerHTML = ""
          resolve(body.data)
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
  const label = element("label", "display:flex;flex-direction:column;gap:4px")
  label.textContent = text
  const input = element("input", "padding:10px;font-size:18px")
  input.value = value
  input.placeholder = placeholder
  label.append(input)
  return { label, input }
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
