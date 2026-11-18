import { betterAuth } from "better-auth"
import { database } from "./db"

// A fixed secret only for `npm run dev`; the published app always has its own real one.
const DEVELOPMENT_SECRET = "development-only-4f9c2b7e1a6d8e3f5c0b9a7d2e4f6c8b"

export function auth(env: Env, request: Request) {
  const google = env.GOOGLE_CLIENT_ID && env.GOOGLE_CLIENT_SECRET
  return betterAuth({
    database: { db: database(env), type: "sqlite" },
    secret: env.BETTER_AUTH_SECRET ?? (import.meta.env.DEV ? DEVELOPMENT_SECRET : undefined),
    baseURL: env.APP_URL ?? new URL(request.url).origin,
    emailAndPassword: { enabled: true },
    socialProviders: google ? { google: { clientId: env.GOOGLE_CLIENT_ID!, clientSecret: env.GOOGLE_CLIENT_SECRET! } } : {},
    telemetry: { enabled: false },
  })
}
