import { Hono } from "hono"
import { auth } from "./auth"
import { database } from "./db"
import { migrate } from "./migrate"

const app = new Hono<{ Bindings: Env }>().basePath("/api")

app.use(async (c, next) => {
  await migrate(c.env.DB)
  await next()
})

app.on(["GET", "POST"], "/auth/*", (c) => auth(c.env, c.req.raw).handler(c.req.raw))

// Example: each signed-in user's own notes. Replace it with the app's real routes.
app.get("/notes", async (c) => {
  const session = await auth(c.env, c.req.raw).api.getSession({ headers: c.req.raw.headers })
  if (!session) return c.json({ error: "Sign in first." }, 401)
  const notes = await database(c.env)
    .selectFrom("note")
    .select(["id", "text", "created_at"])
    .where("user_id", "=", session.user.id)
    .orderBy("created_at", "desc")
    .execute()
  return c.json(notes)
})

app.post("/notes", async (c) => {
  const session = await auth(c.env, c.req.raw).api.getSession({ headers: c.req.raw.headers })
  if (!session) return c.json({ error: "Sign in first." }, 401)
  const body = await c.req.json<{ text?: unknown }>().catch(() => ({ text: undefined }))
  const text = typeof body.text === "string" ? body.text.trim().slice(0, 2000) : ""
  if (!text) return c.json({ error: "Write something first." }, 400)
  const note = { id: crypto.randomUUID(), user_id: session.user.id, text, created_at: Date.now() }
  await database(c.env).insertInto("note").values(note).execute()
  return c.json(note, 201)
})

export default app
