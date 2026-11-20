// Dictation for the composer's mic button: the app sends the recording as the browser made it (AAC
// in MP4 on iPhones, Opus in WebM elsewhere; older desktops send 16 kHz mono WAV) and the gateway
// transcribes it with Whisper on Cloudflare Workers AI (about $0.0005 per audio minute).

/** Free dictation per user per day, in seconds. */
export const DAILY_SECONDS = 30 * 60
/** The longest single recording the desktop sends, in seconds. */
export const MAX_SECONDS = 60
// 16 kHz, 16-bit, mono.
const BYTES_PER_SECOND = 32_000
const WAV_HEADER = 44

/**
 * The length of a base64 recording in seconds, or a reason it can't be transcribed. The app states
 * the length of a compressed recording; it never counts for less than the recording would last as
 * WAV, so a WAV needs no stated length and understating one gains nothing.
 */
export function recording(audio: unknown, stated?: unknown): { seconds: number } | { problem: string } {
  if (typeof audio !== "string" || audio.length === 0) return { problem: "No recording was sent." }
  if (!/^[A-Za-z0-9+/]+={0,2}$/.test(audio)) return { problem: "The recording isn't valid." }
  const padding = audio.endsWith("==") ? 2 : audio.endsWith("=") ? 1 : 0
  const bytes = (audio.length * 3) / 4 - padding
  const seconds = Math.max(
    1,
    Math.ceil((bytes - WAV_HEADER) / BYTES_PER_SECOND),
    typeof stated === "number" && Number.isFinite(stated) ? Math.ceil(stated) : 0,
  )
  if (seconds > MAX_SECONDS + 1) return { problem: `Recordings can be at most ${MAX_SECONDS} seconds.` }
  return { seconds }
}
