import { Button } from "@opencode/ui/button"
import { Icon } from "@opencode/ui/icon"
import { IconButton } from "@opencode/ui/icon-button"
import { useDialog } from "@opencode/ui/context/dialog"
import { For, Show } from "solid-js"
import { useLanguage } from "@/runtime/i18n/language"
import type { Account } from "./api"
import { ChannelSettingsDialog, ConnectDialog } from "./channels"
import { ChannelAvatar } from "./icons"
import type { MarketModel } from "./model"

export function ChannelSidebar(props: {
  model: MarketModel
  compact: boolean
  onManage: () => void
  onCreate: () => void
}) {
  const language = useLanguage()
  const dialog = useDialog()
  const selected = (account: Account) =>
    props.model.state.selectedAccounts === null ||
    props.model.state.selectedAccounts.includes(account.connectedAccountId)
  const toggle = (account: Account) => {
    const current =
      props.model.state.selectedAccounts ?? props.model.state.accounts.map((item) => item.connectedAccountId)
    props.model.setState(
      "selectedAccounts",
      selected(account)
        ? current.filter((id) => id !== account.connectedAccountId)
        : [...current, account.connectedAccountId],
    )
  }
  const rows = (groupId?: string) => props.model.state.accounts.filter((account) => account.groupId === groupId)
  const renderAccount = (account: Account) => (
    <div class="flex min-w-0 items-center gap-1 rounded-lg px-1 py-1 hover:bg-v2-overlay-simple-overlay-hover">
      <button
        type="button"
        aria-pressed={selected(account)}
        title={account.name}
        aria-label={account.name}
        onClick={() => toggle(account)}
        class="flex min-w-0 flex-1 items-center gap-2 rounded-md p-1 text-start aria-[pressed=false]:opacity-40"
      >
        <ChannelAvatar account={account} size="small" />
        <Show when={!props.compact}>
          <span class="min-w-0 flex-1 truncate text-[13px]">{account.targetName ?? account.name}</span>
        </Show>
      </button>
      <Show when={!props.compact}>
        <Show when={account.needsTarget && !account.needsReconnect}>
          <button
            type="button"
            class="text-[11px] text-v2-state-fg-warning"
            onClick={() => dialog.show(() => <ChannelSettingsDialog model={props.model} account={account} />)}
          >
            {language.t("market.channels.targetMissing")}
          </button>
        </Show>
        <Show when={account.needsReconnect}>
          <button
            type="button"
            class="text-[11px] text-v2-state-fg-warning"
            onClick={() =>
              dialog.show(() => (
                <ConnectDialog model={props.model} channel={account.channel} reconnect={account.connectedAccountId} />
              ))
            }
          >
            {language.t("market.channels.reconnect")}
          </button>
        </Show>
        <IconButton
          icon={<Icon name="outline-dots" />}
          variant="ghost-muted"
          aria-label={language.t("market.channels.settings", { name: account.name })}
          onClick={() => dialog.show(() => <ChannelSettingsDialog model={props.model} account={account} />)}
        />
      </Show>
    </div>
  )
  return (
    <aside
      class="flex shrink-0 flex-col gap-3 border-e border-v2-border-border-muted p-2"
      style={{ width: props.compact ? "60px" : "220px" }}
      aria-label={language.t("market.nav.channels")}
    >
      <Show when={!props.compact}>
        <div class="px-2 pt-1 text-[13px] font-medium">{language.t("market.nav.channels")}</div>
      </Show>
      <Button variant="neutral" icon="plus" onClick={props.onManage} aria-label={language.t("market.sidebar.add")}>
        <Show when={!props.compact}>{language.t("market.sidebar.add")}</Show>
      </Button>
      <Button
        variant="contrast"
        icon="plus"
        onClick={props.onCreate}
        disabled={!props.model.state.accounts.length}
        aria-label={language.t("market.channels.create")}
      >
        <Show when={!props.compact}>{language.t("market.channels.create")}</Show>
      </Button>
      <Show when={!props.compact}>
        <button
          type="button"
          class="px-2 text-start text-[12px] text-v2-text-text-muted"
          onClick={() =>
            props.model.setState("selectedAccounts", props.model.state.selectedAccounts === null ? [] : null)
          }
        >
          {language.t(props.model.state.selectedAccounts === null ? "market.sidebar.clear" : "market.sidebar.all")}
        </button>
      </Show>
      <div class="min-h-0 flex-1 overflow-y-auto" data-scrollable>
        <For each={rows()}>{renderAccount}</For>
        <For each={props.model.state.groups}>
          {(group) => (
            <details open class="mt-3">
              <summary class="cursor-pointer truncate px-2 text-[12px] text-v2-text-text-muted" title={group.name}>
                {props.compact ? group.name.slice(0, 1) : group.name}
              </summary>
              <For each={rows(group._id)}>{renderAccount}</For>
            </details>
          )}
        </For>
        <Show when={!props.model.state.accounts.length && !props.compact}>
          <p class="px-2 py-6 text-[13px] text-v2-text-text-muted">{language.t("market.analytics.empty")}</p>
        </Show>
      </div>
      <Button
        variant="ghost"
        onClick={() => props.model.setState("collapsed", !props.model.state.collapsed)}
        aria-label={language.t("market.sidebar.collapse")}
      >
        <Icon name={props.compact ? "chevron-right" : "chevron-left"} />
      </Button>
    </aside>
  )
}
