// Social network tokens are stored encrypted with AES-256-GCM. The key is MARKET_TOKEN_KEY
// (32 random bytes, base64), set once by set-market-secrets.ps1. Web Crypto works in both
// Convex runtimes, so the OAuth callback and the Node publishing worker share this module.

let cached: Promise<CryptoKey> | undefined

function key() {
  return (cached ??= (async () => {
    const raw = process.env.MARKET_TOKEN_KEY?.trim()
    if (!raw) throw new Error("Marketing is not configured: MARKET_TOKEN_KEY is missing.")
    const bytes = decode(raw)
    if (bytes.length !== 32) throw new Error("MARKET_TOKEN_KEY must be 32 bytes of base64.")
    return await crypto.subtle.importKey("raw", bytes, "AES-GCM", false, ["encrypt", "decrypt"])
  })())
}

export async function seal(value: unknown) {
  const iv = crypto.getRandomValues(new Uint8Array(12))
  const data = await crypto.subtle.encrypt({ name: "AES-GCM", iv }, await key(), new TextEncoder().encode(JSON.stringify(value)))
  return `v1.${encode(iv)}.${encode(new Uint8Array(data))}`
}

export async function open<T>(sealed: string): Promise<T> {
  const [version, iv, data] = sealed.split(".")
  if (version !== "v1" || !iv || !data) throw new Error("Stored credentials are unreadable. Reconnect this channel.")
  const plain = await crypto.subtle.decrypt({ name: "AES-GCM", iv: decode(iv) }, await key(), decode(data))
  return JSON.parse(new TextDecoder().decode(plain)) as T
}

/** URL-safe random token for OAuth state and PKCE verifiers. */
export function random(bytes = 32) {
  return encode(crypto.getRandomValues(new Uint8Array(bytes))).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "")
}

export async function challenge(verifier: string) {
  const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(verifier)))
  return encode(digest).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "")
}

function encode(bytes: Uint8Array) {
  return btoa(Array.from(bytes, (byte) => String.fromCharCode(byte)).join(""))
}

function decode(text: string) {
  return Uint8Array.from(atob(text), (char) => char.charCodeAt(0))
}
