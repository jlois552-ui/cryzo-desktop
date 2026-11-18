import { Kysely } from "kysely"
import { D1Dialect } from "kysely-d1"

// Types for the app's own tables. Add a table here when a migration creates it.
export interface Database {
  note: {
    id: string
    user_id: string
    text: string
    created_at: number
  }
}

// All data access goes through Kysely, so moving to Postgres later means swapping this dialect.
export function database(env: Env) {
  return new Kysely<Database>({ dialect: new D1Dialect({ database: env.DB }) })
}
