import { type Component, createResource, For, onCleanup, onMount, Show } from "solid-js"
import { createStore } from "solid-js/store"
import { Button } from "@opencode/ui/button"
import { Switch } from "@opencode/ui/switch"
import { renderSVG } from "@opencode/util/qr"
import { useLanguage } from "@/runtime/i18n/language"
import { type RemotePairing, usePlatform } from "@/runtime/platform/platform"
import { SettingsList } from "@/settings/list"
import { SettingsRow } from "@/settings/row"
import "@/settings/settings.css"

export const SettingsRemote: Component = () => {
  const language = useLanguage()
  const platform = usePlatform()
  const [status, { mutate, refetch }] = createResource(() => platform.getRemoteControl?.() ?? Promise.resolve(null))
  const [devices, devicesControl] = createResource(
    () => (status.state === "ready" || status.state === "refreshing") && status.latest?.enabled,
    () => platform.listRemoteDevices?.() ?? Promise.resolve([]),
  )
  const [state, setState] = createStore({ busy: false, error: "", pairing: undefined as RemotePairing | undefined })
  // Guarded so the page never suspends; it shows nothing until the host has answered once.
  const current = () => (status.state === "ready" || status.state === "refreshing" ? status.latest : undefined)
  const paired = () => (devices.state === "ready" || devices.state === "refreshing" ? (devices.latest ?? []) : [])

  // The link reconnects and devices come and go on their own, so the page follows them while open.
  onMount(() => {
    const timer = setInterval(() => {
      void refetch()
      if (current()?.enabled) void devicesControl.refetch()
    }, 3000)
    onCleanup(() => clearInterval(timer))
  })

  const run = async (work: () => Promise<void>) => {
    setState({ busy: true, error: "" })
    await work().catch((error: unknown) => setState("error", error instanceof Error ? error.message : String(error)))
    setState("busy", false)
  }

  const toggle = (enabled: boolean) =>
    run(async () => {
      const next = await platform.setRemoteControl!(enabled)
      mutate(next)
      setState("pairing", undefined)
      void devicesControl.refetch()
    })

  const pair = () =>
    run(async () => {
      setState("pairing", await platform.pairRemoteDevice!())
    })

  const revoke = (id: string) =>
    run(async () => {
      await platform.revokeRemoteDevice!(id)
      void devicesControl.refetch()
    })

  const statusText = () => {
    const value = current()
    if (!value?.enabled) return ""
    if (value.error && !value.connected) return value.error
    if (!value.connected) return language.t("settings.remote.status.connecting")
    if (value.clients > 0) return language.plural("settings.remote.status.clients", value.clients)
    return language.t("settings.remote.status.connected")
  }

  const qr = () => {
    const pairing = state.pairing
    if (!pairing) return ""
    return renderSVG(pairing.url ?? pairing.code, { border: 2 })
  }

  return (
    <>
      <div class="settings-tab-header">
        <div class="settings-tab-header-row">
          <div class="flex flex-col gap-1">
            <h2 class="settings-tab-title">{language.t("settings.tab.remote")}</h2>
            <span class="text-11-regular text-v2-text-text-muted">{language.t("settings.remote.description")}</span>
          </div>
        </div>
      </div>

      <div class="settings-tab-body settings-tab-body--sectioned">
        <Show
          when={current()}
          fallback={
            <Show when={status.state === "ready" && status.latest === null}>
              <span class="text-13-regular text-v2-text-text-muted">{language.t("settings.remote.unavailable")}</span>
            </Show>
          }
        >
          {(value) => (
            <>
              <div class="settings-section">
                <SettingsList>
                  <SettingsRow
                    title={language.t("settings.remote.row.enabled.title")}
                    description={language.t("settings.remote.row.enabled.description")}
                  >
                    <div data-action="settings-remote-enabled">
                      <Switch checked={value().enabled} disabled={state.busy} onChange={(enabled) => void toggle(enabled)} />
                    </div>
                  </SettingsRow>
                </SettingsList>
                <Show when={statusText()}>
                  <span class="text-12-regular text-v2-text-text-muted" role="status">
                    {statusText()}
                  </span>
                </Show>
                <Show when={state.error}>
                  <span class="text-12-regular text-v2-state-fg-danger" role="alert">
                    {state.error}
                  </span>
                </Show>
              </div>

              <Show when={value().enabled}>
                <div class="settings-section">
                  <h3 class="settings-section-title">{language.t("settings.remote.section.pair")}</h3>
                  <span class="text-12-regular text-v2-text-text-muted">
                    {language.t("settings.remote.pair.description")}
                  </span>
                  <Show
                    when={state.pairing}
                    fallback={
                      <div>
                        <Button size="normal" variant="neutral" disabled={state.busy} onClick={() => void pair()}>
                          {language.t("settings.remote.pair.button")}
                        </Button>
                      </div>
                    }
                  >
                    {(pairing) => (
                      <div class="flex items-center gap-4">
                        <div
                          class="size-32 shrink-0 rounded-[8px] bg-white p-1 [&>svg]:size-full"
                          role="img"
                          aria-label={language.t("settings.remote.pair.qr")}
                          innerHTML={qr()}
                        />
                        <div class="flex flex-col gap-2">
                          <code class="font-mono text-[24px] leading-8 tracking-[2px] text-v2-text-text-base">
                            {pairing().code}
                          </code>
                          <div>
                            <Button size="normal" variant="ghost-muted" disabled={state.busy} onClick={() => void pair()}>
                              {language.t("settings.remote.pair.again")}
                            </Button>
                          </div>
                        </div>
                      </div>
                    )}
                  </Show>
                </div>

                <div class="settings-section">
                  <h3 class="settings-section-title">{language.t("settings.remote.section.devices")}</h3>
                  <Show
                    when={paired().length > 0}
                    fallback={
                      <span class="text-12-regular text-v2-text-text-muted">
                        {language.t("settings.remote.devices.empty")}
                      </span>
                    }
                  >
                    <SettingsList>
                      <For each={paired()}>
                        {(device) => (
                          <SettingsRow
                            title={device.name}
                            description={
                              device.lastSeenAt
                                ? language.t("settings.remote.devices.lastSeen", {
                                    time: new Date(device.lastSeenAt).toLocaleString(),
                                  })
                                : language.t("settings.remote.devices.never")
                            }
                          >
                            <Button
                              size="normal"
                              variant="ghost-muted"
                              disabled={state.busy}
                              aria-label={language.t("settings.remote.devices.removeLabel", { name: device.name })}
                              onClick={() => void revoke(device.id)}
                            >
                              {language.t("settings.remote.devices.remove")}
                            </Button>
                          </SettingsRow>
                        )}
                      </For>
                    </SettingsList>
                  </Show>
                </div>
              </Show>
            </>
          )}
        </Show>
      </div>
    </>
  )
}
