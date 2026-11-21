# This app

A Cryzo app: a React site with a Cloudflare Worker behind it, a SQLite database (Cloudflare D1) and sign-in (Better Auth). It runs the same way on your computer, on Cryzo's hosting and on your own Cloudflare account.

## Commands

- Install once with the `shell` tool: program `npm`, args `["install"]`.
- Run the app with the `preview` tool: program `npm`, args `["run", "dev"]`. It starts the site, API and a local database, and shows the app in Cryzo's browser pane as soon as it answers. Never start it with `shell` (it would stop after 120 seconds), and don't open the local address in another browser. Use `preview_logs` to read errors.
- `npm run check` type-checks the site and the Worker.
- `npm run build` writes the publishable app to `dist/`. Run it when the app works, then call the `publish` tool. It checks the app and shows the user a Publish card to click; you can't publish yourself.

## Where things go

- `src/client/` is the React site. Style with Tailwind classes. `auth.ts` is the sign-in client: `authClient.useSession()`, `authClient.signIn.email(...)`, `authClient.signOut()`.
- `src/worker/index.ts` is the API (Hono). Every route lives under `/api`; any other path serves the site, so client-side routes work on reload.
- `src/worker/db.ts` holds the table types and `database(env)`, a Kysely query builder. Use it for all data access instead of raw `env.DB` calls.
- `src/worker/auth.ts` configures Better Auth. In a route, `await auth(c.env, c.req.raw).api.getSession({ headers: c.req.raw.headers })` returns the signed-in user or `null`.
- `migrations/` holds numbered SQL files. They run automatically, once each, on the first request (locally and when published).
- The notes feature (`/api/notes`, `Notes` in `App.tsx`, `0002_notes.sql`) is an example. Replace it with what the user asked for.

## Adding file uploads

Only add file storage when the app needs uploads (photos, documents, audio, video). Records belong in the database. When it does:

1. Add `"r2_buckets": [{ "binding": "FILES", "bucket_name": "app" }]` to `wrangler.jsonc`, and `FILES?: R2Bucket` to `Env`. Cryzo creates the bucket on the next publish, and `npm run dev` simulates one locally.
2. Serve files through Worker routes under `/api`, never from a public bucket. Check the session first, and store each user's files under a `${session.user.id}/` key prefix.
3. If `env.FILES` is missing, answer 503 with a clear message. That happens when the host has no file storage yet.
4. Limit upload size (for example 10 MB) and save the original file name and type with `put(key, file.stream(), { httpMetadata: { contentType }, customMetadata: { name } })`.
5. When serving, show images inline (not SVG). Send every other file with `Content-Disposition: attachment` and `X-Content-Type-Options: nosniff`, so an uploaded HTML file can never run on the app's address.
6. If files need searching or sorting, keep their details (owner, name, size, key) in a database table too.

## Local and live data

`npm run dev` uses its own database (and file storage, if the app has uploads) on this computer, in `.wrangler/`. The published app has separate live ones. Nothing you run locally can read or change live data, so test freely.

- To test with real data, ask the user to click **Download data** next to the app under **Publish**. It saves a `.sql` file to their Downloads folder. To load it locally, stop the dev server, delete `.wrangler/state`, run `npx wrangler d1 execute DB --local --file=<that file>`, and start `npm run dev` again. The file holds the app's users and their data, so never copy it into the project or publish it.
- Live data changes only through migrations, and each publish is checked against the live database:
  - Publishing is blocked if a migration that already ran live was edited or deleted.
  - A new migration that drops a table or column, or deletes rows, needs the user's confirmation. Prefer adding columns and tables over removing them.

## Rules

- **Changing the database:** add a new file such as `migrations/0003_tasks.sql` and the matching type in `db.ts`. Never edit or delete a migration that may have run; write a new one instead. Use SQLite syntax and end each statement with `;` at the end of a line.
- **Protecting data:** every route that reads or changes a user's data must check the session and filter by `session.user.id`. Never trust a user ID sent from the browser.
- **Secrets:** never put API keys or passwords in files, not even in `.env`, and never ask the user to paste one into the chat. Read them from `env` in the Worker (add each to `Env` in `src/worker/env.d.ts`, optional, so the app still runs without it). Tell the user the exact name to add under **Publish → Secrets** next to the published app. The value goes straight to the app's host and nobody can view it again, including you. Cryzo blocks publishing when it finds a secret in the project.
- **Google sign-in** turns on when the `GOOGLE_CLIENT_ID` and `GOOGLE_CLIENT_SECRET` secrets exist. Then add a button that calls `authClient.signIn.social({ provider: "google" })`. The Google OAuth client needs `<app address>/api/auth/callback/google` as its redirect URI.
- Secrets only exist on the published app. Code that needs one must handle it being missing in `npm run dev`, for example by showing that the feature isn't set up yet.
- **Keep it portable:** stay on Workers-compatible packages (no native Node addons, no filesystem writes) and keep `wrangler.jsonc` as the single source of the app's settings.

## Publishing

The user publishes with Cryzo's Publish button, which gives the app its own address on cryzo.dev, a database, a file bucket if `wrangler.jsonc` declares one, `APP_URL` and `BETTER_AUTH_SECRET`. Make sure `npm run build` succeeds first, then call the `publish` tool: it lists anything that blocks publishing, and otherwise shows the user a Publish card in the chat.

## Moving to the user's own setup

When the user wants to run the app on their own accounts:

1. **Their own Cloudflare account:** run `npx wrangler login`, then `npx wrangler d1 create <name>`, and put the printed id in `wrangler.jsonc`. If the app has uploads, run `npx wrangler r2 bucket create <name>` and put that name under `r2_buckets`. Then run `npx wrangler secret put BETTER_AUTH_SECRET` (the user types a new random value), set `APP_URL` under `vars` and run `npm run deploy`. To keep the data, import an export of the Cryzo database with `npx wrangler d1 execute <name> --remote --file=<export.sql>`.
2. **Postgres (for example Neon or Supabase):** change `database()` in `db.ts` to Kysely's Postgres dialect, change `type: "sqlite"` to `"postgres"` in `auth.ts`, and convert the migrations to Postgres syntax.
