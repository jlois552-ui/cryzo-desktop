import { createMemo, Show } from "solid-js"
import { useI18n } from "@opencode/ui/context/i18n"
import { BasicTool } from "../components/basic-tool"
import { computerStep } from "./computer-step"
import type { ToolProps } from "./tool-renderer"

export { isComputerTool } from "./computer-name"
export { computerStep } from "./computer-step"

/** A computer-use call as one plain line; opening it shows what the tool returned. */
export function ComputerTool(props: ToolProps) {
  const i18n = useI18n()
  const line = createMemo(() => computerStep(i18n, props.tool, props.input, props.output))

  return (
    <BasicTool
      {...props}
      icon="window-cursor"
      hasContent={!!props.output}
      trigger={{ title: line() }}
    >
      <Show when={props.output}>
        <div data-component="tool-output" data-scrollable tabIndex={0} role="region" aria-label={line()}>
          <pre class="whitespace-pre-wrap break-words font-mono text-[12px] leading-4 text-v2-text-text-muted">
            {props.output}
          </pre>
        </div>
      </Show>
    </BasicTool>
  )
}
