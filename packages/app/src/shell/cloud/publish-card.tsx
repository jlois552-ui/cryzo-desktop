import { Show } from "solid-js"
import { createStore } from "solid-js/store"
import { Button } from "@opencode/ui/button"
import { useDialog } from "@opencode/ui/context/dialog"
import { ToolRegistry, type ToolProps } from "@opencode/session-ui/tool-renderer"
import { useLanguage } from "@/runtime/i18n/language"
import { usePlatform } from "@/runtime/platform/platform"
import { useServerSDK } from "@/runtime/server/client"
import { cloudRequest } from "./api"
import { DialogPublish } from "./publish-dialog"

/**
 * The card the agent's `publish` tool leaves in the chat once the app passes Cryzo's checks. The
 * agent never publishes: going live is always the user's click here, through the Publish dialog.
 */
function PublishCard(props: ToolProps) {
  const ready = () => readyApp(props.output)

  return (
    <Show when={props.status === "completed" && ready()}>
      {(app) => <Card directory={app().directory} slug={app().slug} url={app().url} />}
    </Show>
  )
}

function Card(props: { directory: string; slug: string; url: string | null }) {
  const sdk = useServerSDK()
  const platform = usePlatform()
  const language = useLanguage()
  const dialog = useDialog()
  const [state, setState] = createStore({ url: props.url })

  // After the dialog closes the app may be live, or live at a new address.
  const refresh = () =>
    cloudRequest({
      url: sdk.url,
      password: sdk.http.password,
      platform,
      directory: props.directory,
      path: "app",
      fallback: language.t("cloud.error"),
    })
      .then((data) => setState("url", typeof data.url === "string" ? data.url : null))
      .catch(() => undefined)

  return (
    <div
      data-component="publish-card"
      class="flex flex-col gap-3 rounded-xl border-[0.5px] border-v2-border-border-muted bg-v2-background-bg-base p-4 shadow-[var(--v2-elevation-raised)]"
    >
      <div class="flex flex-col gap-1">
        <p class="text-[14px] font-medium leading-[var(--line-height-base)] text-v2-text-text-base">
          {state.url ? language.t("publish.live") : language.t("publish.card.title")}
        </p>
        <p class="text-[13px] leading-[var(--line-height-compact)] text-v2-text-text-muted">
          {language.t("publish.card.description")}
        </p>
      </div>
      <p class="truncate font-mono text-[13px] leading-[var(--line-height-compact)] text-v2-text-text-base">
        {state.url ?? props.slug}
      </p>
      <div class="flex gap-2">
        <Button
          variant="contrast"
          size="large"
          onClick={() => dialog.show(() => <DialogPublish directory={props.directory} />, () => void refresh())}
        >
          {state.url ? language.t("publish.card.update") : language.t("publish.action")}
        </Button>
        <Show when={state.url}>
          {(url) => (
            <Button variant="neutral" size="large" onClick={() => platform.openExternal(url())}>
              {language.t("publish.open")}
            </Button>
          )}
        </Show>
      </div>
    </div>
  )
}

/** The tool's result as the engine wrote it, or nothing when it isn't a ready app. */
function readyApp(output: string | undefined) {
  // The engine always returns a JSON object for a ready app; anything else is no card.
  const parsed: unknown = output?.startsWith("{") ? JSON.parse(output) : undefined
  if (!parsed || typeof parsed !== "object") return
  const app = parsed as { ready?: unknown; slug?: unknown; url?: unknown; directory?: unknown }
  if (app.ready !== true || typeof app.slug !== "string" || typeof app.directory !== "string") return
  return { slug: app.slug, directory: app.directory, url: typeof app.url === "string" ? app.url : null }
}

ToolRegistry.register({ name: "publish", render: PublishCard })
