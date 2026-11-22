import { createReactiveTimelineProjection } from "@opencode/session-ui/timeline/projection"

export { reuseTimelineRows } from "@opencode/session-ui/timeline/projection"

export function createTimelineProjection(input: Parameters<typeof createReactiveTimelineProjection>[0]) {
  // Finished turns fold their steps into one "Worked for …" line above the answer, as in the ChatGPT app.
  return createReactiveTimelineProjection({ foldFinishedTurns: () => true, ...input })
}
