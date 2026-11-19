import { Button } from "@opencode/ui/button"
import { Icon } from "@opencode/ui/icon"
import { Tooltip } from "@opencode/ui/tooltip"
import { useLanguage } from "@/runtime/i18n/language"
import { useServerSDK } from "@/runtime/server/client"
import { projectSettingsKey, useSettings } from "@/settings/model"
import { showToast } from "@/shell/notifications/toast"
import { useWorkspaceLocation } from "@/workspaces/location"

/**
 * Ask first or Full access, like the permissions control under ChatGPT's composer. It is the
 * project's auto-approve setting (Settings → General keeps the default for other projects), so
 * the choice is remembered. Each request still gets its own approval from the engine; Full access
 * just answers them automatically.
 */
export function FullAccessChip() {
  const settings = useSettings()
  const sdk = useServerSDK()
  const location = useWorkspaceLocation()
  const language = useLanguage()
  const project = () => projectSettingsKey(sdk.scope, location().current?.project.id)
  const full = () => settings.permissions.autoApproves(project())

  const toggle = () => {
    const key = project()
    const next = !full()
    if (!key) settings.permissions.setAutoApproveDefault(next)
    else
      settings.permissions.setAutoApproveOverride(
        key,
        next === settings.permissions.autoApproveDefault() ? undefined : next,
      )
    showToast(
      next
        ? { title: language.t("access.full.on.title"), description: language.t("access.full.on.description") }
        : { title: language.t("access.full.off.title"), description: language.t("access.full.off.description") },
    )
  }

  return (
    <Tooltip
      value={full() ? language.t("access.full.tooltip") : language.t("access.ask.tooltip")}
      placement="top"
      openDelay={500}
    >
      <Button
        type="button"
        size="small"
        variant="ghost-muted"
        data-action="composer-access"
        aria-pressed={full()}
        class={full() ? "gap-1 px-1.5 text-v2-state-fg-warning" : "gap-1 px-1.5"}
        onClick={toggle}
      >
        <Icon name={full() ? "warning" : "lock"} size="small" />
        {full() ? language.t("access.full") : language.t("access.ask")}
      </Button>
    </Tooltip>
  )
}
