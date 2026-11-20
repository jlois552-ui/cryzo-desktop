import { onCleanup, Show } from "solid-js"
import { createStore } from "solid-js/store"
import { Button } from "@opencode/ui/button"
import { useDialog } from "@opencode/ui/context/dialog"
import { Icon } from "@opencode/ui/icon"
import { IconButton } from "@opencode/ui/icon-button"
import { Loader } from "@opencode/ui/loader"
import { Tooltip } from "@opencode/ui/tooltip"
import { useLanguage } from "@/runtime/i18n/language"
import { usePlatform } from "@/runtime/platform/platform"
import { authTokenFromCredentials } from "@/runtime/server/api"
import { useServerSDK } from "@/runtime/server/client"
import { showToast } from "@/shell/notifications/toast"
import { cloudRequest } from "@/shell/cloud/api"

const MAX_SECONDS = 60
const RATE = 16_000
const BARS = 24

/**
 * Dictation, like the mic next to ChatGPT's send button: click to record and the composer shows a
 * live waveform with a timer, cancel and done; the words land in the composer for review and are
 * never sent on their own. Esc cancels. Audio goes to Whisper through the Cryzo gateway and is not
 * kept.
 */
export function ComposerMic(props: { onText: (text: string) => void }) {
  const sdk = useServerSDK()
  const platform = usePlatform()
  const language = useLanguage()
  const dialog = useDialog()
  const [state, setState] = createStore({ recording: false, busy: false, seconds: 0 })
  const session: {
    recorder?: MediaRecorder
    chunks: Blob[]
    timer?: ReturnType<typeof setInterval>
    cancelled: boolean
    audio?: AudioContext
    frame?: number
  } = {
    chunks: [],
    cancelled: false,
  }
  let canvas: HTMLCanvasElement | undefined

  const fail = (description: string) => showToast({ title: language.t("mic.error"), description })

  // Dictation runs on the Cryzo gateway, so a signed-out user signs in first instead of talking
  // for a minute and then getting an error.
  const signedIn = async () => {
    const password = sdk.http.password
    const response = await (platform.fetch ?? fetch)(`${sdk.url}/api/cryzo/account`, {
      headers: password ? { Authorization: `Basic ${authTokenFromCredentials({ password })}` } : {},
    }).catch(() => undefined)
    const data = (await response?.json().catch(() => undefined)) as { signedIn?: boolean } | undefined
    return !!data?.signedIn
  }

  const signIn = async () => {
    const { DialogConnectProvider, useProviderConnectController } = await import("@/providers/connect/dialog")
    const controller = useProviderConnectController()
    controller.select("cryzo")
    void dialog.show(() => <DialogConnectProvider controller={controller} standalone />)
  }

  // Bars that follow the voice, drawn in the text color so they suit both themes.
  const waveform = (stream: MediaStream) => {
    const audio = new AudioContext()
    const analyser = audio.createAnalyser()
    analyser.fftSize = 256
    audio.createMediaStreamSource(stream).connect(analyser)
    session.audio = audio
    const levels = new Uint8Array(analyser.frequencyBinCount)
    const draw = () => {
      session.frame = requestAnimationFrame(draw)
      const context = canvas?.getContext("2d")
      if (!canvas || !context) return
      analyser.getByteFrequencyData(levels)
      const ratio = devicePixelRatio || 1
      const width = canvas.clientWidth * ratio
      const height = canvas.clientHeight * ratio
      if (canvas.width !== width || canvas.height !== height) Object.assign(canvas, { width, height })
      context.clearRect(0, 0, width, height)
      context.fillStyle = getComputedStyle(canvas).color
      const step = width / BARS
      for (let bar = 0; bar < BARS; bar++) {
        const level = levels[Math.floor((bar / BARS) * levels.length * 0.6)] / 255
        const size = Math.max(2 * ratio, level * height)
        context.fillRect(bar * step + step * 0.2, (height - size) / 2, step * 0.6, size)
      }
    }
    draw()
  }

  const start = async () => {
    if (!(await signedIn())) return void signIn()
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
    waveform(stream)
    session.timer = setInterval(() => {
      setState("seconds", (seconds) => seconds + 1)
      if (state.seconds >= MAX_SECONDS) stop()
    }, 1000)
    addEventListener("keydown", escape)
  }

  const stop = (cancel = false) => {
    clearInterval(session.timer)
    if (session.frame !== undefined) cancelAnimationFrame(session.frame)
    void session.audio?.close().catch(() => undefined)
    session.audio = undefined
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

  const label = () => (state.busy ? language.t("mic.busy") : language.t("mic.start"))
  const time = () => `${Math.floor(state.seconds / 60)}:${String(state.seconds % 60).padStart(2, "0")}`

  return (
    <Show
      when={state.recording}
      fallback={
        <Tooltip value={label()} placement="top" openDelay={500}>
          <Button
            type="button"
            size="small"
            variant="ghost-muted"
            data-action="composer-mic"
            aria-label={label()}
            disabled={state.busy}
            class="me-1 px-1.5"
            onClick={() => void start()}
          >
            <Show when={state.busy} fallback={<Icon name="mic" size="small" />}>
              <Loader class="size-4" />
            </Show>
          </Button>
        </Tooltip>
      }
    >
      <div data-slot="composer-dictation" class="me-1 flex items-center gap-1" role="group" aria-label={language.t("mic.recording")}>
        <Tooltip value={language.t("mic.cancel")} placement="top" openDelay={500}>
          <IconButton
            type="button"
            size="small"
            variant="ghost-muted"
            icon={<Icon name="close" size="small" />}
            aria-label={language.t("mic.cancel")}
            onClick={() => stop(true)}
          />
        </Tooltip>
        <canvas ref={canvas} aria-hidden="true" class="h-5 w-16 text-v2-text-text-base" />
        <span class="w-8 text-12-regular tabular-nums text-v2-text-text-muted">{time()}</span>
        <Tooltip value={language.t("mic.stop")} placement="top" openDelay={500}>
          <IconButton
            type="button"
            size="small"
            variant="ghost-muted"
            data-action="composer-mic"
            icon={<Icon name="check" size="small" />}
            aria-label={language.t("mic.stop")}
            onClick={() => stop()}
          />
        </Tooltip>
      </div>
    </Show>
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
