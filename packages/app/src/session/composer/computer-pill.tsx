import type { SessionMessageInfo } from "@opencode/client/promise"
import { Spinner } from "@opencode/ui/spinner"
import { useLanguage } from "@/runtime/i18n/language"

/** Whether the turn in progress has used computer use (a `desktop_*` tool) since the last user message. */
export function usingComputer(messages: readonly SessionMessageInfo[]) {
  return messages
    .slice(messages.findLastIndex((message) => message.type === "user") + 1)
    .some(
      (message) =>
        message.type === "assistant" &&
        message.content.some((item) => item.type === "tool" && item.name.startsWith("desktop_")),
    )
}

// Shown for the whole turn rather than per tool call, so it doesn't flicker while the model thinks between actions.
// The global Ctrl+Alt+Esc works from any app, since the user is usually watching another window, not Cryzo.
export function SessionComputerPill(props: { onStop: () => void }) {
  const language = useLanguage()

  return (
    <div
      data-component="session-computer-pill"
      role="status"
      class="pointer-events-auto flex h-7 min-w-0 shrink items-center gap-2 rounded-[12px] border-[0.5px] border-v2-border-border-muted bg-v2-background-bg-base ps-2.5 pe-1 text-[13px] font-[440] leading-text-compact tracking-[-0.04px] whitespace-nowrap text-v2-text-text-base shadow-[var(--v2-elevation-raised)]"
    >
      <Spinner width={14} height={14} class="shrink-0" />
      <span class="min-w-0 truncate">{language.t("session.computer.active")}</span>
      <span class="min-w-0 truncate text-v2-text-text-muted">{language.t("session.computer.hint")}</span>
      <button
        type="button"
        class="h-5 shrink-0 rounded-[8px] px-2 text-[12px] font-medium text-v2-text-text-base hover:bg-v2-background-bg-subtle"
        onClick={() => props.onStop()}
      >
        {language.t("session.computer.stop")}
      </button>
    </div>
  )
}
