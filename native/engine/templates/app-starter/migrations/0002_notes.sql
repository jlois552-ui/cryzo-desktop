-- The example notes table. Never edit a migration that has run; add a new numbered file instead.
CREATE TABLE note (
  id TEXT PRIMARY KEY,
  user_id TEXT NOT NULL REFERENCES "user" (id) ON DELETE CASCADE,
  text TEXT NOT NULL,
  created_at INTEGER NOT NULL
);
CREATE INDEX note_user_id_created_at_idx ON note (user_id, created_at);
