import { Button } from "@opencode/ui/button"
import { Loader } from "@opencode/ui/loader"
import { For, Show, createMemo, createEffect, on, onMount } from "solid-js"
import { createStore, reconcile } from "solid-js/store"
import { useLanguage } from "@/runtime/i18n/language"
import type { Report } from "./api"
import { ConnectDialog } from "./channels"
import { useDialog } from "@opencode/ui/context/dialog"
import { ChannelAvatar } from "./icons"
import type { MarketModel } from "./model"

const ranges = [7, 30, 90] as const

/** Provider metric keys such as `page_post_engagements` shown as words. */
const label = (key: string) =>
  key
    .replace(/([a-z])([A-Z])/g, "$1 $2")
    .replace(/[_-]/g, " ")
    .replace(/\b\w/g, (letter) => letter.toUpperCase())

export function Analytics(props: { model: MarketModel }) {
  const language = useLanguage()
  const dialog = useDialog()
  const [state, setState] = createStore({
    days: 30 as (typeof ranges)[number],
    loading: false,
    reports: [] as Report[],
    loaded: false,
  })

  const load = async (refresh = false) => {
    setState("loading", true)
    const reports = await props.model.run(() => props.model.api.analytics(state.days, refresh))
    setState({ loading: false, loaded: true })
    if (reports) setState("reports", reconcile(reports, { key: "connectedAccountId" }))
  }
  onMount(() => void load())
  createEffect(
    on(
      () => props.model.state.analyticsRevision,
      () => void load(true),
      { defer: true },
    ),
  )

  const number = (value: number) =>
    new Intl.NumberFormat(language.intl(), {
      maximumFractionDigits: 1,
      notation: value >= 100_000 ? "compact" : "standard",
    }).format(value)

  return (
    <div class="flex min-h-0 min-w-0 flex-1 flex-col gap-4">
      <div class="flex items-center gap-3">
        <Show when={state.loading}>
          <Loader class="size-4" />
        </Show>
        <div class="flex-1" />
        <div class="flex h-9 items-center rounded-[8px] border border-v2-border-border-muted p-0.5" role="tablist">
          <For each={ranges}>
            {(days) => (
              <button
                type="button"
                role="tab"
                aria-selected={state.days === days}
                class="h-full rounded-[6px] px-3 text-[13px] text-v2-text-text-muted aria-selected:bg-v2-background-bg-layer-03 aria-selected:text-v2-text-text-base"
                onClick={() => {
                  setState("days", days)
                  void load()
                }}
              >
                {language.t(`market.analytics.range.${days}`)}
              </button>
            )}
          </For>
        </div>
        <Button variant="neutral" icon="refresh" disabled={state.loading} onClick={() => void load(true)}>
          {language.t("market.analytics.refresh")}
        </Button>
      </div>
      <div class="min-h-0 flex-1 overflow-y-auto" data-scrollable>
        <Show
          when={props.model.state.accounts.length}
          fallback={
            <p class="py-10 text-center text-[13px] text-v2-text-text-muted">{language.t("market.analytics.empty")}</p>
          }
        >
          <div class="grid grid-cols-[repeat(auto-fill,minmax(340px,1fr))] gap-4">
            <For each={props.model.state.accounts}>
              {(account) => {
                const report = createMemo(() =>
                  state.reports.find((item) => item.connectedAccountId === account.connectedAccountId),
                )
                return (
                  <section class="flex flex-col gap-3 rounded-[12px] bg-v2-background-bg-layer-01 p-4">
                    <div class="flex items-center gap-3">
                      <ChannelAvatar account={account} />
                      <span class="min-w-0 flex-1 truncate text-[14px] font-medium leading-5 text-v2-text-text-base">
                        {account.name}
                      </span>
                    </div>
                    <Show
                      when={report()}
                      fallback={
                        <Show when={!state.loaded}>
                          <Loader class="size-4" />
                        </Show>
                      }
                    >
                      {(report) => (
                        <>
                          <Show when={report().status !== "ready"}>
                            <p class="text-[12px] leading-4 text-v2-text-text-muted">
                              {report().permissionReason === "approval_pending"
                                ? language.t("market.analytics.approval")
                                : (report().note ?? language.t(`market.analytics.status.${report().status}`))}
                            </p>
                          </Show>
                          <Show when={report().permissionReason === "reconnect_required"}>
                            <Button
                              variant="neutral"
                              size="small"
                              onClick={() =>
                                dialog.show(() => (
                                  <ConnectDialog
                                    model={props.model}
                                    channel={account.channel}
                                    reconnect={account.connectedAccountId}
                                  />
                                ))
                              }
                            >
                              {language.t("market.analytics.reconnect")}
                            </Button>
                          </Show>
                          <div class="text-[11px] text-v2-text-text-faint">
                            {language.t("market.analytics.updated", {
                              time: new Intl.DateTimeFormat(language.intl(), {
                                hour: "numeric",
                                minute: "2-digit",
                              }).format(report().updatedAt),
                            })}
                          </div>
                          <div class="grid grid-cols-2 gap-2">
                            <For each={report().metrics}>
                              {(metric) => (
                                <div class="rounded-[8px] bg-v2-background-bg-base px-3 py-2">
                                  <div class="truncate text-[11px] leading-4 text-v2-text-text-faint">
                                    {label(metric.key)}
                                  </div>
                                  <div class="text-[18px] font-medium leading-7 text-v2-text-text-base">
                                    {number(metric.value)}
                                  </div>
                                  <div class="text-[10px] leading-4 text-v2-text-text-faint">
                                    {language.t(`market.analytics.window.${metric.window}`)}
                                  </div>
                                </div>
                              )}
                            </For>
                          </div>
                          <For each={report().series.slice(0, 2)}>
                            {(series) => <Sparkline label={label(series.key)} points={series.points} />}
                          </For>
                        </>
                      )}
                    </Show>
                  </section>
                )
              }}
            </For>
          </div>
        </Show>
      </div>
    </div>
  )
}

function Sparkline(props: { label: string; points: { date: string; value: number }[] }) {
  const path = createMemo(() => {
    const values = props.points.map((point) => point.value)
    const max = Math.max(...values, 1)
    const min = Math.min(...values, 0)
    const span = max - min || 1
    return props.points
      .map((point, index) => {
        const x = props.points.length > 1 ? (index / (props.points.length - 1)) * 300 : 150
        const y = 56 - ((point.value - min) / span) * 52
        return `${index ? "L" : "M"}${x.toFixed(1)},${y.toFixed(1)}`
      })
      .join(" ")
  })
  return (
    <div class="flex flex-col gap-1">
      <span class="text-[11px] leading-4 text-v2-text-text-faint">{props.label}</span>
      <svg
        viewBox="0 0 300 60"
        class="h-16 w-full text-v2-icon-icon-accent"
        preserveAspectRatio="none"
        aria-hidden="true"
      >
        <path d={path()} fill="none" stroke="currentColor" stroke-width="2" vector-effect="non-scaling-stroke" />
      </svg>
    </div>
  )
}
