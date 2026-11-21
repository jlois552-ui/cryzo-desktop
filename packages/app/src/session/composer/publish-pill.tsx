import { createResource, Show } from "solid-js"
import { useDialog } from "@opencode/ui/context/dialog"
import { useLanguage } from "@/runtime/i18n/language"
import { usePlatform } from "@/runtime/platform/platform"
import { useServerSDK } from "@/runtime/server/client"
import { cloudRequest } from "@/shell/cloud/api"
import { DialogPublish } from "@/shell/cloud/publish-dialog"

/**
 * Publish, above the composer, for a project that is a Cryzo app with its own database and
 * sign-in. It checks again each time the agent stops, since a build may have just finished.
 */
export function SessionPublishPill(props: { directory: string; idle: boolean }) {
  const sdk = useServerSDK()
  const platform = usePlatform()
  const language = useLanguage()
  const dialog = useDialog()

  const [app, { refetch }] = createResource(
    () => (props.idle ? props.directory : undefined),
    (directory) =>
      cloudRequest({
        url: sdk.url,
        password: sdk.http.password,
        platform,
        directory,
        path: "app",
        fallback: language.t("cloud.error"),
      })
        .then((data) => ({
          directory,
          app: data.app === true,
          url: typeof data.url === "string" ? data.url : undefined,
        }))
        .catch(() => ({ directory, app: false, url: undefined })),
  )

  // Never suspend the composer lane; show nothing until the first answer for this folder.
  const current = () => {
    const loaded = app.state === "ready" || app.state === "refreshing"
    const value = loaded ? app.latest : undefined
    return value?.directory === props.directory && value.app ? value : undefined
  }

  return (
    <Show when={current()}>
      {(info) => (
        <button
          type="button"
          data-component="session-publish-pill"
          class="pointer-events-auto flex h-7 min-w-0 shrink items-center gap-2 rounded-[12px] border-[0.5px] border-v2-border-border-muted bg-v2-background-bg-base px-2.5 text-[13px] font-[440] leading-text-compact tracking-[-0.04px] whitespace-nowrap text-v2-text-text-base shadow-[var(--v2-elevation-raised)] hover:bg-v2-background-bg-subtle"
          onClick={() => dialog.show(() => <DialogPublish directory={props.directory} />, () => void refetch())}
        >
          <Show when={info().url}>
            <span class="size-1.5 shrink-0 rounded-full bg-v2-state-fg-success" aria-hidden="true" />
          </Show>
          <span class="min-w-0 truncate">
            {info().url ? language.t("publish.pill.live") : language.t("publish.button")}
          </span>
        </button>
      )}
    </Show>
  )
}
