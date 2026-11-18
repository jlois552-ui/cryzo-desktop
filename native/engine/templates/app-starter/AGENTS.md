# This app

A Cryzo app: a React site with a Cloudflare Worker behind it, a SQLite database (Cloudflare D1) and sign-in (Better Auth). It runs the same way on your computer, on Cryzo's hosting and on your own Cloudflare account.

## Commands

- `npm install` once, then `npm run dev` to run the whole app (site, API and a local database) at http://localhost:5173.
- `npm run check` type-checks the site and the Worker.
- `npm run build` writes the publishable app to `dist/`. Run it before the user publishes.

## Where things go

- `src/client/` is the React site. Style with Tailwind classes. `auth.ts` is the sign-in client: `authClient.useSession()`, `authClient.signIn.email(...)`, `authClient.signOut()`.
- `src/worker/index.ts` is the API (Hono). Every route lives under `/api`; any other path serves the site, so client-side routes work on reload.
- `src/worker/db.ts` holds the table types and `database(env)`, a Kysely query builder. Use it for all data access instead of raw `env.DB` calls.
- `src/worker/auth.ts` configures Better Auth. In a route, `await auth(c.env, c.req.raw).api.getSession({ headers: c.req.raw.headers })` returns the signed-in user or `null`.
- `migrations/` holds numbered SQL files. They run automatically, once each, on the first request (locally and when published).
- The notes feature (`/api/notes`, `Notes` in `App.tsx`, `0002_notes.sql`) is an example. Replace it with what the user asked for.

## Rules

- **Changing the database:** add a new file such as `migrations/0003_tasks.sql` and the matching type in `db.ts`. Never edit or delete a migration that may have run; write a new one instead. Use SQLite syntax and end each statement with `;` at the end of a line.
- **Protecting data:** every route that reads or changes a user's data must check the session and filter by `session.user.id`. Never trust a user ID sent from the browser.
- **Secrets:** never put API keys or passwords in files, not even in `.env`, and never ask the user to paste one into the chat. Read them from `env` in the Worker (add each to `Env` in `src/worker/env.d.ts`, optional, so the app still runs without it). Tell the user the exact name to add under **Publish → Secrets** next to the published app. The value goes straight to the app's host and nobody can view it again, including you. Cryzo blocks publishing when it finds a secret in the project.
- **Google sign-in** turns on when the `GOOGLE_CLIENT_ID` and `GOOGLE_CLIENT_SECRET` secrets exist. Then add a button that calls `authClient.signIn.social({ provider: "google" })`. The Google OAuth client needs `<app address>/api/auth/callback/google` as its redirect URI.
- Secrets only exist on the published app. Code that needs one must handle it being missing in `npm run dev`, for example by showing that the feature isn't set up yet.
- **Keep it portable:** stay on Workers-compatible packages (no native Node addons, no filesystem writes) and keep `wrangler.jsonc` as the single source of the app's settings.

## Publishing

The user publishes with Cryzo's Publish button, which gives the app its own address on cryzo.dev, a database, `APP_URL` and `BETTER_AUTH_SECRET`. Make sure `npm run build` succeeds first.

## Moving to the user's own setup

When the user wants to run the app on their own accounts:

1. **Their own Cloudflare account:** run `npx wrangler login`, then `npx wrangler d1 create <name>`, and put the printed id in `wrangler.jsonc`. Then run `npx wrangler secret put BETTER_AUTH_SECRET` (the user types a new random value), set `APP_URL` under `vars` and run `npm run deploy`. To keep the data, import an export of the Cryzo database with `npx wrangler d1 execute <name> --remote --file=<export.sql>`.
2. **Postgres (for example Neon or Supabase):** change `database()` in `db.ts` to Kysely's Postgres dialect, change `type: "sqlite"` to `"postgres"` in `auth.ts`, and convert the migrations to Postgres syntax.
