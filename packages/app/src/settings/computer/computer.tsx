import { type Component, createResource, For, Show } from "solid-js"
import { createStore } from "solid-js/store"
import { Button } from "@opencode/ui/button"
import { Select } from "@opencode/ui/select"
import { Switch } from "@opencode/ui/switch"
import { useLanguage } from "@/runtime/i18n/language"
import { type ComputerUseSettings, usePlatform } from "@/runtime/platform/platform"
import { SettingsList } from "@/settings/list"
import { SettingsRow } from "@/settings/row"
import "@/settings/settings.css"

const approvals = [
  { id: "always", label: "settings.computer.approval.always" },
  { id: "per_app", label: "settings.computer.approval.perApp" },
  { id: "never", label: "settings.computer.approval.never" },
] as const

export const SettingsComputer: Component = () => {
  const language = useLanguage()
  const platform = usePlatform()
  const [saved, { mutate }] = createResource(() => platform.getComputerUse?.() ?? Promise.resolve(null))
  const [draft, setDraft] = createStore({ app: "" })
  // Guarded so the page never suspends; it shows nothing until the settings file has been read.
  const current = () => (saved.state === "ready" || saved.state === "refreshing" ? saved.latest : undefined)

  const update = (change: Partial<ComputerUseSettings>) => {
    const before = current()

    if (!before) return
    const next = { ...before, ...change }
    mutate(next)
    void platform.setComputerUse?.(next).catch(() => mutate(before))
  }

  const addApp = () => {
    const app = draft.app.trim()
    const settings = current()

    if (!app || !settings) return
    setDraft("app", "")

    if (settings.blockedApps.some((item) => item.toLowerCase() === app.toLowerCase())) return
    update({ blockedApps: [...settings.blockedApps, app] })
  }

  return (
    <>
      <div class="settings-tab-header">
        <div class="settings-tab-header-row">
          <div class="flex flex-col gap-1">
            <h2 class="settings-tab-title">{language.t("settings.tab.computer")}</h2>
            <span class="text-11-regular text-v2-text-text-muted">{language.t("settings.computer.description")}</span>
          </div>
        </div>
      </div>

      <div class="settings-tab-body settings-tab-body--sectioned">
        <Show
          when={current()}
          fallback={
            <Show when={saved.state === "ready" && saved.latest === null}>
              <span class="text-13-regular text-v2-text-text-muted">{language.t("settings.computer.unavailable")}</span>
            </Show>
          }
        >
          {(settings) => (
            <>
              <div class="settings-section">
                <SettingsList>
                  <SettingsRow
                    title={language.t("settings.computer.row.enabled.title")}
                    description={language.t("settings.computer.row.enabled.description")}
                  >
                    <div data-action="settings-computer-enabled">
                      <Switch checked={settings().enabled} onChange={(enabled) => update({ enabled })} />
                    </div>
                  </SettingsRow>

                  <SettingsRow
                    title={language.t("settings.computer.row.approval.title")}
                    description={language.t("settings.computer.row.approval.description")}
                  >
                    <Select
                      data-action="settings-computer-approval"
                      options={[...approvals]}
                      current={approvals.find((option) => option.id === settings().approval)}
                      value={(option) => option.id}
                      label={(option) => language.t(option.label)}
                      onSelect={(option) => option && update({ approval: option.id })}
                      disabled={!settings().enabled}
                      placement="bottom-end"
                      gutter={6}
                    />
                  </SettingsRow>

                  <SettingsRow
                    title={language.t("settings.computer.row.passwords.title")}
                    description={language.t("settings.computer.row.passwords.description")}
                  >
                    <div data-action="settings-computer-passwords">
                      <Switch
                        checked={settings().typeIntoPasswords}
                        disabled={!settings().enabled}
                        onChange={(typeIntoPasswords) => update({ typeIntoPasswords })}
                      />
                    </div>
                  </SettingsRow>

                  <SettingsRow
                    title={language.t("settings.computer.row.stop.title")}
                    description={language.t("settings.computer.row.stop.description")}
                  >
                    <kbd class="rounded-[6px] border border-v2-border-border-base px-2 py-0.5 font-mono text-12-regular text-v2-text-text-base">
                      {language.t("settings.computer.row.stop.keys")}
                    </kbd>
                  </SettingsRow>
                </SettingsList>
              </div>

              <div class="settings-section">
                <h3 class="settings-section-title">{language.t("settings.computer.section.blocked")}</h3>
                <span class="text-12-regular text-v2-text-text-muted">
                  {language.t("settings.computer.blocked.description")}
                </span>
                <SettingsList>
                  <For each={settings().blockedApps}>
                    {(app) => (
                      <SettingsRow title={app} description="">
                        <Button
                          size="normal"
                          variant="ghost-muted"
                          aria-label={language.t("settings.computer.blocked.remove", { app })}
                          onClick={() => update({ blockedApps: settings().blockedApps.filter((item) => item !== app) })}
                        >
                          {language.t("settings.computer.blocked.removeButton")}
                        </Button>
                      </SettingsRow>
                    )}
                  </For>
                </SettingsList>
                <form
                  class="flex items-center gap-2"
                  onSubmit={(event) => {
                    event.preventDefault()
                    addApp()
                  }}
                >
                  <input
                    class="h-8 min-w-0 flex-1 rounded-[8px] border border-v2-border-border-base bg-v2-background-bg-base px-3 text-13-regular text-v2-text-text-base"
                    value={draft.app}
                    placeholder={language.t("settings.computer.blocked.placeholder")}
                    aria-label={language.t("settings.computer.blocked.placeholder")}
                    onInput={(event) => setDraft("app", event.currentTarget.value)}
                  />
                  <Button type="submit" size="normal" variant="neutral" icon="plus" disabled={!draft.app.trim()}>
                    {language.t("settings.computer.blocked.add")}
                  </Button>
                </form>
              </div>
            </>
          )}
        </Show>
      </div>
    </>
  )
}
