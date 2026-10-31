import { Button } from "@opencode/ui/button"
import { For, Show, createMemo } from "solid-js"
import { useLanguage } from "@/runtime/i18n/language"
import type { Post } from "./api"
import { ChannelAvatar, MarketIcon } from "./icons"
import { startOfDay, type MarketModel, type Section } from "./model"

const day = 86_400_000

/** Wix's Marketing Home: drafts the agent prepared wait here for approval; nothing goes live until approved. */
export function Overview(props: {
  model: MarketModel
  onPlan: () => void
  onOpen: (post: Post) => void
  onSection: (section: Section) => void
}) {
  const language = useLanguage()
  const today = startOfDay(Date.now())
  const feed = () => props.model.state.feed
  const waiting = createMemo(() =>
    feed()
      .filter((post) => post.status === "draft" && post.scheduledFor >= today)
      .sort((a, b) => a.scheduledFor - b.scheduledFor),
  )
  const stats = createMemo(() => [
    { key: "drafts", value: waiting().length },
    { key: "scheduled", value: feed().filter((post) => post.status === "scheduled" && post.scheduledFor < Date.now() + 7 * day).length },
    { key: "published", value: feed().filter((post) => post.status === "published" && post.scheduledFor >= Date.now() - 7 * day).length },
    { key: "attention", value: feed().filter((post) => post.status === "failed").length },
  ])
  const attention = () => props.model.state.accounts.filter((account) => account.needsReconnect || account.needsTarget).length
  const date = (time: number) => new Intl.DateTimeFormat(language.intl(), { weekday: "short", month: "short", day: "numeric", hour: "numeric", minute: "2-digit" }).format(time)

  const approve = async (post: Post) => {
    // A draft whose time has passed needs a new time first.
    if (post.scheduledFor <= Date.now()) return props.onOpen(post)
    const done = await props.model.run(() => props.model.api.updatePost({ postId: post._id, status: "scheduled" }))
    if (done !== undefined) await props.model.loadPosts()
  }
  const discard = async (post: Post) => {
    const done = await props.model.run(() => props.model.api.deletePost(post._id))
    if (done !== undefined) await props.model.loadPosts()
  }

  return (
    <div class="flex flex-col gap-5">
      <div class="grid gap-3 md:grid-cols-2">
        <section class="flex flex-col gap-3 rounded-[12px] border border-v2-border-border-muted p-5">
          <MarketIcon name="sparkle" class="size-6 text-v2-icon-icon-accent" />
          <h3 class="text-[16px] font-medium leading-6 text-v2-text-text-base">{language.t("market.overview.ideas.title")}</h3>
          <p class="text-[13px] leading-5 text-v2-text-text-muted">{language.t("market.overview.ideas.description")}</p>
          <div>
            <Button variant="contrast" disabled={!props.model.state.accounts.length} onClick={props.onPlan}>
              {language.t("market.overview.ideas.action")}
            </Button>
          </div>
        </section>
        <section class="flex flex-col gap-3 rounded-[12px] border border-v2-border-border-muted p-5">
          <MarketIcon name="channels" class="size-6 text-v2-icon-icon-muted" />
          <h3 class="text-[16px] font-medium leading-6 text-v2-text-text-base">{language.t("market.overview.channels.title")}</h3>
          <Show
            when={props.model.state.accounts.length}
            fallback={<p class="text-[13px] leading-5 text-v2-text-text-muted">{language.t("market.channels.empty")}</p>}
          >
            <div class="flex flex-wrap gap-1.5">
              <For each={props.model.state.accounts}>{(account) => <ChannelAvatar account={account} />}</For>
            </div>
            <Show when={attention()}>
              <p class="text-[12px] leading-4 text-v2-state-fg-warning">{language.t("market.overview.channels.attention")}</p>
            </Show>
          </Show>
          <div>
            <Button variant="neutral" onClick={() => props.onSection("channels")}>
              {language.t("market.overview.channels.action")}
            </Button>
          </div>
        </section>
      </div>

      <div class="grid grid-cols-2 gap-3 md:grid-cols-4">
        <For each={stats()}>
          {(stat) => (
            <div class="rounded-[12px] bg-v2-background-bg-layer-01 px-4 py-3">
              <div class="text-[22px] font-medium leading-8 text-v2-text-text-base">{stat.value}</div>
              <div class="text-[12px] leading-4 text-v2-text-text-muted">{language.t(`market.overview.stat.${stat.key}`)}</div>
            </div>
          )}
        </For>
      </div>

      <section class="flex flex-col gap-2">
        <h3 class="text-[15px] font-medium leading-6 text-v2-text-text-base">{language.t("market.overview.waiting")}</h3>
        <Show when={waiting().length} fallback={<p class="text-[13px] leading-5 text-v2-text-text-muted">{language.t("market.overview.waitingEmpty")}</p>}>
          <For each={waiting()}>
            {(post) => (
              <div class="flex items-center gap-3 rounded-[10px] border border-v2-border-border-muted px-3 py-2.5">
                <span class="flex shrink-0 -space-x-2">
                  <For each={post.targets.slice(0, 3)}>
                    {(target) => <Show when={props.model.account(target.connectedAccountId)}>{(account) => <ChannelAvatar account={account()} size="small" />}</Show>}
                  </For>
                </span>
                <div class="min-w-0 flex-1">
                  <div class="truncate text-[13px] leading-5 text-v2-text-text-base">{post.content || language.t("market.post.untitled")}</div>
                  <div class="text-[12px] leading-4 text-v2-text-text-muted">{date(post.scheduledFor)}</div>
                </div>
                <Button variant="ghost" size="small" onClick={() => void discard(post)}>
                  {language.t("market.overview.discard")}
                </Button>
                <Button variant="ghost" size="small" onClick={() => props.onOpen(post)}>
                  {language.t("market.overview.edit")}
                </Button>
                <Button variant="contrast" size="small" onClick={() => void approve(post)}>
                  {language.t("market.overview.approve")}
                </Button>
              </div>
            )}
          </For>
        </Show>
      </section>
    </div>
  )
}
