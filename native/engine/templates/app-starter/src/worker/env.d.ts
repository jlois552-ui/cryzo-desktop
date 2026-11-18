// What the app gets from its host. Cryzo sets DB, APP_URL and BETTER_AUTH_SECRET on publish;
// the Google values are secrets the user adds to turn on Google sign-in.
interface Env {
  DB: D1Database
  ASSETS: Fetcher
  APP_URL?: string
  BETTER_AUTH_SECRET?: string
  GOOGLE_CLIENT_ID?: string
  GOOGLE_CLIENT_SECRET?: string
}
