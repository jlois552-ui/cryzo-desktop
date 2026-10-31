import { Button } from "@opencode/ui/button"
import { useDialog } from "@opencode/ui/context/dialog"
import { Dialog, DialogTitle } from "@opencode/ui/dialog"
import { Icon } from "@opencode/ui/icon"
import { IconButton } from "@opencode/ui/icon-button"
import { Loader } from "@opencode/ui/loader"
import { For, Show, createMemo, onCleanup, onMount } from "solid-js"
import { createStore } from "solid-js/store"
import { useLanguage } from "@/runtime/i18n/language"
import { usePlatform } from "@/runtime/platform/platform"
import { channels, type Account, type Channel } from "./api"
import { ChannelAvatar, MarketIcon, NetworkBadge } from "./icons"
import type { MarketModel } from "./model"

export function ChannelsColumn(props: { model: MarketModel; onCreate: () => void; onGenerate: () => void }) {
  const language = useLanguage()
  const dialog = useDialog()
  const sections = createMemo(() => [
    { id: undefined as string | undefined, name: language.t("market.channels.ungrouped"), color: undefined as string | undefined },
    ...props.model.state.groups.map((group) => ({ id: group._id as string | undefined, name: group.name, color: group.color as string | undefined })),
  ])
  const members = (groupId: string | undefined) => props.model.state.accounts.filter((account) => account.groupId === groupId)

  return (
    <Show
      when={!props.model.state.collapsed}
      fallback={
        <div class="flex w-12 shrink-0 flex-col items-center gap-2 border-e border-v2-border-border-muted py-4">
          <IconButton
            variant="ghost-muted"
            icon={<Icon name="outline-chevron-right" />}
            aria-label={language.t("market.channels.expand")}
            onClick={() => props.model.setState("collapsed", false)}
          />
          <For each={props.model.state.accounts}>{(account) => <ChannelAvatar account={account} size="small" />}</For>
        </div>
      }
    >
      <aside class="flex w-[280px] shrink-0 flex-col gap-4 border-e border-v2-border-border-muted px-4 py-5">
        <div class="flex h-8 items-center justify-between">
          <h2 class="text-[18px] font-medium leading-7 text-v2-text-text-base">{language.t("market.channels.title")}</h2>
          <IconButton
            variant="ghost-muted"
            icon={<Icon name="outline-arrow-left" />}
            aria-label={language.t("market.channels.collapse")}
            onClick={() => props.model.setState("collapsed", true)}
          />
        </div>
        <Button
          variant="neutral"
          size="large"
          class="w-full"
          icon="plus"
          onClick={() => dialog.show(() => <AddChannelDialog model={props.model} />)}
        >
          {language.t("market.channels.add")}
        </Button>
        <div class="flex gap-2">
          <Button
            variant="contrast"
            size="large"
            class="min-w-0 flex-1"
            icon="plus"
            disabled={!props.model.state.accounts.length}
            onClick={props.onCreate}
          >
            {language.t("market.channels.create")}
          </Button>
          <button
            type="button"
            class="grid size-10 shrink-0 place-items-center rounded-[8px] bg-v2-background-bg-accent hover:opacity-90"
            style={{ color: "#ffffff" }}
            aria-label={language.t("market.channels.generate")}
            title={language.t("market.channels.generate")}
            onClick={props.onGenerate}
          >
            <MarketIcon name="sparkle" />
          </button>
        </div>
        <div class="-mx-1 min-h-0 flex-1 overflow-y-auto px-1" data-scrollable>
          <Show
            when={props.model.state.accounts.length}
            fallback={<p class="px-1 text-[13px] leading-5 text-v2-text-text-muted">{language.t("market.channels.empty")}</p>}
          >
            <For each={sections()}>
              {(section) => (
                <Show when={members(section.id).length}>
                  <div class="mb-3">
                    <Show when={section.id}>
                      <div class="mb-1 flex items-center gap-2 px-1 text-[12px] font-medium leading-4 text-v2-text-text-faint">
                        <span class="size-2 rounded-full" style={{ "background-color": section.color }} />
                        {section.name}
                      </div>
                    </Show>
                    <For each={members(section.id)}>
                      {(account) => (
                        <div class="group flex h-12 items-center gap-3 rounded-[8px] px-1 hover:bg-v2-overlay-simple-overlay-hover">
                          <ChannelAvatar account={account} />
                          <div class="min-w-0 flex-1">
                            <div class="truncate text-[14px] leading-5 text-v2-text-text-base">{account.name}</div>
                            <Show when={(account.channel === "facebook" || account.channel === "instagram") && !account.targetId}>
                              <div class="truncate text-[12px] leading-4 text-v2-state-fg-warning">
                                {language.t("market.channels.targetMissing")}
                              </div>
                            </Show>
                          </div>
                          <IconButton
                            variant="ghost-muted"
                            icon={<Icon name="outline-dots" />}
                            aria-label={language.t("market.channels.settings", { name: account.name })}
                            onClick={() => dialog.show(() => <ChannelSettingsDialog model={props.model} account={account} />)}
                          />
                        </div>
                      )}
                    </For>
                  </div>
                </Show>
              )}
            </For>
          </Show>
        </div>
      </aside>
    </Show>
  )
}

export const networkName = (language: ReturnType<typeof useLanguage>, channel: Channel) => language.t(`market.network.${channel}`)

function AddChannelDialog(props: { model: MarketModel }) {
  const language = useLanguage()
  const platform = usePlatform()
  const dialog = useDialog()
  const [state, setState] = createStore({ waiting: undefined as Channel | undefined, before: 0 })
  let timer: ReturnType<typeof setInterval> | undefined
  onCleanup(() => clearInterval(timer))

  const connect = async (channel: Channel) => {
    const url = await props.model.run(() => props.model.api.connect(channel))
    if (!url) return
    platform.openExternal(url)
    setState({ waiting: channel, before: props.model.state.accounts.length })
    clearInterval(timer)
    // The connection finishes in the browser; poll until Composio reports the new account.
    let attempts = 0
    timer = setInterval(async () => {
      attempts++
      await props.model.loadAccounts(true)
      if (props.model.state.accounts.length > state.before || attempts >= 36) {
        clearInterval(timer)
        if (props.model.state.accounts.length > state.before) dialog.close()
        else setState("waiting", undefined)
      }
    }, 5000)
  }

  return (
    <Dialog fit containerClass="!w-[min(calc(100vw_-_32px),460px)] !rounded-xl">
      <div class="flex flex-col gap-4 p-6">
        <DialogTitle>
          <span class="text-[18px] font-medium leading-7 text-v2-text-text-base">{language.t("market.connect.title")}</span>
        </DialogTitle>
        <Show
          when={state.waiting}
          fallback={
            <div class="grid grid-cols-3 gap-3">
              <For each={channels}>
                {(channel) => (
                  <button
                    type="button"
                    class="flex flex-col items-center gap-2 rounded-[10px] border border-v2-border-border-muted px-3 py-4 text-[13px] leading-4 text-v2-text-text-base hover:bg-v2-overlay-simple-overlay-hover"
                    onClick={() => void connect(channel)}
                  >
                    <NetworkBadge channel={channel} class="size-9 text-[14px] leading-none" />
                    {networkName(language, channel)}
                  </button>
                )}
              </For>
            </div>
          }
        >
          {(channel) => (
            <div class="flex flex-col items-center gap-3 py-6 text-center">
              <NetworkBadge channel={channel()} class="size-10 text-[15px] leading-none" />
              <p class="text-[13px] leading-5 text-v2-text-text-muted">{language.t("market.connect.description")}</p>
              <div class="flex items-center gap-2 text-[13px] leading-5 text-v2-text-text-base">
                <Loader class="size-4" />
                {language.t("market.connect.waiting")}
              </div>
              <Button variant="ghost" onClick={() => void props.model.loadAccounts(true)}>
                {language.t("market.channels.refresh")}
              </Button>
            </div>
          )}
        </Show>
      </div>
    </Dialog>
  )
}

function ChannelSettingsDialog(props: { model: MarketModel; account: Account }) {
  const language = useLanguage()
  const dialog = useDialog()
  const needsTarget = props.account.channel === "facebook" || props.account.channel === "instagram"
  const [state, setState] = createStore({
    name: props.account.name,
    groupId: props.account.groupId ?? "",
    targetId: props.account.targetId ?? "",
    newGroup: "",
    targets: undefined as { id: string; name: string }[] | undefined,
    saving: false,
    confirm: false,
  })

  onMount(async () => {
    if (!needsTarget) return
    const targets = await props.model.run(() => props.model.api.targets(props.account.connectedAccountId))
    setState("targets", targets ?? [])
    if (!state.targetId && targets?.length === 1) setState("targetId", targets[0].id)
  })

  const save = async () => {
    setState("saving", true)
    const groupId = state.newGroup.trim()
      ? await props.model.run(() => props.model.api.saveGroup({ name: state.newGroup, color: palette[props.model.state.groups.length % palette.length] }))
      : state.groupId || null
    const saved = await props.model.run(() =>
      props.model.api.updateAccount({
        connectedAccountId: props.account.connectedAccountId,
        name: state.name,
        groupId: groupId ?? null,
        ...(needsTarget && state.targetId ? { targetId: state.targetId } : {}),
      }),
    )
    setState("saving", false)
    if (saved === undefined) return
    await props.model.loadAccounts()
    dialog.close()
  }

  const disconnect = async () => {
    setState("saving", true)
    const done = await props.model.run(() => props.model.api.disconnect(props.account.connectedAccountId))
    setState("saving", false)
    if (done === undefined) return
    await props.model.loadAccounts()
    dialog.close()
  }

  const field = "h-9 w-full rounded-[8px] border border-v2-border-border-base bg-v2-background-bg-base px-3 text-[13px] text-v2-text-text-base"

  return (
    <Dialog fit containerClass="!w-[min(calc(100vw_-_32px),440px)] !rounded-xl">
      <div class="flex flex-col gap-4 p-6">
        <div class="flex items-center gap-3">
          <ChannelAvatar account={props.account} />
          <DialogTitle>
            <span class="text-[16px] font-medium leading-6 text-v2-text-text-base">{props.account.name}</span>
          </DialogTitle>
        </div>
        <label class="flex flex-col gap-1.5 text-[12px] leading-4 text-v2-text-text-muted">
          {language.t("market.channels.name")}
          <input class={field} value={state.name} maxLength={100} onInput={(event) => setState("name", event.currentTarget.value)} />
        </label>
        <Show when={needsTarget}>
          <label class="flex flex-col gap-1.5 text-[12px] leading-4 text-v2-text-text-muted">
            {language.t(props.account.channel === "facebook" ? "market.channels.page" : "market.channels.instagramAccount")}
            <Show when={state.targets} fallback={<Loader class="size-4" />}>
              {(targets) => (
                <Show
                  when={targets().length}
                  fallback={<span class="text-[13px] leading-5 text-v2-state-fg-warning">{language.t("market.channels.noTargets")}</span>}
                >
                  <select class={field} value={state.targetId} onChange={(event) => setState("targetId", event.currentTarget.value)}>
                    <option value="">{language.t("market.channels.choose")}</option>
                    <For each={targets()}>{(target) => <option value={target.id}>{target.name}</option>}</For>
                  </select>
                </Show>
              )}
            </Show>
          </label>
        </Show>
        <label class="flex flex-col gap-1.5 text-[12px] leading-4 text-v2-text-text-muted">
          {language.t("market.channels.group")}
          <select class={field} value={state.groupId} onChange={(event) => setState("groupId", event.currentTarget.value)}>
            <option value="">{language.t("market.channels.ungrouped")}</option>
            <For each={props.model.state.groups}>{(group) => <option value={group._id}>{group.name}</option>}</For>
          </select>
          <input
            class={field}
            placeholder={language.t("market.channels.newGroup")}
            value={state.newGroup}
            maxLength={60}
            onInput={(event) => setState("newGroup", event.currentTarget.value)}
          />
        </label>
        <div class="mt-2 flex items-center gap-2">
          <Show
            when={state.confirm}
            fallback={
              <Button variant="ghost" class="text-v2-state-fg-danger" onClick={() => setState("confirm", true)}>
                {language.t("market.channels.disconnect")}
              </Button>
            }
          >
            <Button variant="ghost" class="text-v2-state-fg-danger" disabled={state.saving} onClick={() => void disconnect()}>
              {language.t("market.channels.disconnectConfirm")}
            </Button>
          </Show>
          <div class="flex-1" />
          <Button variant="ghost" onClick={() => dialog.close()}>
            {language.t("common.cancel")}
          </Button>
          <Button variant="contrast" disabled={state.saving || !state.name.trim()} onClick={() => void save()}>
            {language.t("common.save")}
          </Button>
        </div>
      </div>
    </Dialog>
  )
}

export const palette = ["#3B82F6", "#8B5CF6", "#EC4899", "#F59E0B", "#10B981", "#EF4444", "#06B6D4"]
