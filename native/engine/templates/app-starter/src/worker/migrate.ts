// Applies the SQL files in /migrations once each, in name order. It records them in the same
// table `wrangler d1 migrations apply` uses, so the app can later move to plain Wrangler.
const files = import.meta.glob<string>("../../migrations/*.sql", { query: "?raw", import: "default", eager: true })
const migrations = Object.entries(files)
  .map(([path, sql]) => ({ name: path.split("/").at(-1)!, sql }))
  .sort((a, b) => a.name.localeCompare(b.name))

let ready: Promise<void> | undefined

export function migrate(db: D1Database) {
  ready ??= run(db).catch((error: unknown) => {
    ready = undefined
    throw error
  })
  return ready
}

async function run(db: D1Database) {
  await db
    .prepare(
      "CREATE TABLE IF NOT EXISTS d1_migrations (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT UNIQUE, applied_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP NOT NULL)",
    )
    .run()
  const applied = await names(db)
  for (const migration of migrations.filter((migration) => !applied.has(migration.name))) {
    const statements = migration.sql
      .split(/;\s*(?:\r?\n|$)/)
      .filter((statement) => statement.replace(/--.*$/gm, "").trim())
      .map((statement) => db.prepare(statement))
    // A batch is one transaction. If another instance applied it first, ours rolls back.
    await db
      .batch([...statements, db.prepare("INSERT INTO d1_migrations (name) VALUES (?)").bind(migration.name)])
      .catch(async (error: unknown) => {
        if (!(await names(db)).has(migration.name)) throw error
      })
  }
}

async function names(db: D1Database) {
  const rows = await db.prepare("SELECT name FROM d1_migrations").all<{ name: string }>()
  return new Set(rows.results.map((row) => row.name))
}
