import { onCleanup, Show } from "solid-js"
import { createStore } from "solid-js/store"
import { Button } from "@opencode/ui/button"
import { Icon } from "@opencode/ui/icon"
import { Tooltip } from "@opencode/ui/tooltip"
import { useLanguage } from "@/runtime/i18n/language"
import { usePlatform } from "@/runtime/platform/platform"
import { useServerSDK } from "@/runtime/server/client"
import { showToast } from "@/shell/notifications/toast"
import { cloudRequest } from "@/shell/cloud/api"

const MAX_SECONDS = 60
const RATE = 16_000

/**
 * Dictation, like the mic next to ChatGPT's send button: click to record, click again to stop,
 * and the words land in the composer. Esc cancels. Audio goes to Whisper through the Cryzo
 * gateway and is not kept.
 */
export function ComposerMic(props: { onText: (text: string) => void }) {
  const sdk = useServerSDK()
  const platform = usePlatform()
  const language = useLanguage()
  const [state, setState] = createStore({ recording: false, busy: false, seconds: 0 })
  const session: {
    recorder?: MediaRecorder
    chunks: Blob[]
    timer?: ReturnType<typeof setInterval>
    cancelled: boolean
  } = {
    chunks: [],
    cancelled: false,
  }

  const fail = (description: string) => showToast({ title: language.t("mic.error"), description })

  const start = async () => {
    const allowed = platform.requestMicrophone ? await platform.requestMicrophone().catch(() => false) : true
    if (!allowed) return fail(language.t("mic.denied"))
    if (!navigator.mediaDevices?.getUserMedia) return fail(language.t("mic.unavailable"))
    // A permission prompt that never shows would leave the button doing nothing; give up instead.
    const stream = await Promise.race([
      navigator.mediaDevices.getUserMedia({ audio: true }),
      new Promise<never>((_, reject) => setTimeout(() => reject(new Error(language.t("mic.timeout"))), 10_000)),
    ]).catch((error: unknown) => {
      fail(error instanceof Error && error.name !== "NotAllowedError" ? error.message : language.t("mic.denied"))
      return undefined
    })
    if (!stream) return
    const recorder = new MediaRecorder(stream)
    session.recorder = recorder
    session.chunks = []
    session.cancelled = false
    recorder.ondataavailable = (event) => session.chunks.push(event.data)
    recorder.onstop = () => {
      stream.getTracks().forEach((track) => track.stop())
      if (!session.cancelled) void transcribe(new Blob(session.chunks, { type: recorder.mimeType }))
    }
    recorder.start()
    setState({ recording: true, seconds: 0 })
    session.timer = setInterval(() => {
      setState("seconds", (seconds) => seconds + 1)
      if (state.seconds >= MAX_SECONDS) stop()
    }, 1000)
    addEventListener("keydown", escape)
  }

  const stop = (cancel = false) => {
    clearInterval(session.timer)
    removeEventListener("keydown", escape)
    session.cancelled = cancel
    if (session.recorder?.state === "recording") session.recorder.stop()
    setState("recording", false)
  }

  function escape(event: KeyboardEvent) {
    if (event.key === "Escape") stop(true)
  }

  const transcribe = async (blob: Blob) => {
    setState("busy", true)
    const data = await wav(blob)
      .then((audio) =>
        cloudRequest({
          url: sdk.url,
          password: sdk.http.password,
          platform,
          directory: "",
          path: "transcribe",
          body: { audio },
          fallback: language.t("mic.error"),
        }),
      )
      .catch((error: unknown) => {
        fail(error instanceof Error ? error.message : String(error))
        return undefined
      })
    setState("busy", false)
    const text = typeof data?.text === "string" ? data.text.trim() : ""
    if (text) props.onText(`${text} `)
  }

  onCleanup(() => stop(true))

  const label = () =>
    state.recording ? language.t("mic.stop") : state.busy ? language.t("mic.busy") : language.t("mic.start")

  return (
    <Tooltip value={label()} placement="top" openDelay={500}>
      <Button
        type="button"
        size="small"
        variant="ghost-muted"
        data-action="composer-mic"
        aria-label={label()}
        aria-pressed={state.recording}
        disabled={state.busy}
        class={state.recording ? "me-1 gap-1 px-1.5 text-v2-state-fg-danger" : "me-1 px-1.5"}
        onClick={() => (state.recording ? stop() : void start())}
      >
        <Icon name="mic" size="small" />
        <Show when={state.recording}>
          <span class="tabular-nums">
            {Math.floor(state.seconds / 60)}:{String(state.seconds % 60).padStart(2, "0")}
          </span>
        </Show>
      </Button>
    </Tooltip>
  )
}

/** The recording as base64 16 kHz mono 16-bit WAV, the format Whisper reads most reliably. */
async function wav(blob: Blob) {
  const context = new AudioContext()
  const decoded = await context.decodeAudioData(await blob.arrayBuffer()).finally(() => context.close())
  const offline = new OfflineAudioContext(1, Math.max(1, Math.ceil(decoded.duration * RATE)), RATE)
  const source = offline.createBufferSource()
  source.buffer = decoded
  source.connect(offline.destination)
  source.start()
  const samples = (await offline.startRendering()).getChannelData(0)
  const view = new DataView(new ArrayBuffer(44 + samples.length * 2))
  const ascii = (offset: number, text: string) =>
    Array.from(text).forEach((char, index) => view.setUint8(offset + index, char.charCodeAt(0)))
  ascii(0, "RIFF")
  view.setUint32(4, 36 + samples.length * 2, true)
  ascii(8, "WAVE")
  ascii(12, "fmt ")
  view.setUint32(16, 16, true)
  view.setUint16(20, 1, true)
  view.setUint16(22, 1, true)
  view.setUint32(24, RATE, true)
  view.setUint32(28, RATE * 2, true)
  view.setUint16(32, 2, true)
  view.setUint16(34, 16, true)
  ascii(36, "data")
  view.setUint32(40, samples.length * 2, true)
  samples.forEach((sample, index) => view.setInt16(44 + index * 2, Math.max(-1, Math.min(1, sample)) * 0x7fff, true))
  const bytes = new Uint8Array(view.buffer)
  const parts: string[] = []
  for (let index = 0; index < bytes.length; index += 0x8000)
    parts.push(String.fromCharCode(...bytes.subarray(index, index + 0x8000)))
  return btoa(parts.join(""))
}
