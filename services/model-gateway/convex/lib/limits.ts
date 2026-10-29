export function limit(input: { calls: number; userActive: boolean; active: number; minuteCalls: number; maxActive: number; maxMinute: number; blockedUntil: number; now: number }) {
  if (!Number.isFinite(input.maxActive) || input.maxActive < 1 || input.maxActive > 100 || !Number.isFinite(input.maxMinute) || input.maxMinute < 1) return "free service unavailable"
  if (input.calls >= 100) return "daily limit reached; resets at 00:00 UTC"
  if (input.userActive) return "one generation at a time"
  if (input.blockedUntil > input.now || input.active >= input.maxActive || input.minuteCalls >= input.maxMinute) return "free service busy; retry shortly"
  return null
}
