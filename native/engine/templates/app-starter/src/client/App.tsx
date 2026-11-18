import { useEffect, useState, type FormEvent } from "react"
import { authClient } from "./auth"

export function App() {
  const session = authClient.useSession()
  return (
    <main className="min-h-screen bg-stone-50 px-4 py-12 text-stone-900 dark:bg-stone-950 dark:text-stone-100">
      <div className="mx-auto flex max-w-md flex-col gap-6">
        <h1 className="text-2xl font-semibold tracking-tight">My app</h1>
        {session.isPending ? (
          <p className="text-sm text-stone-500">Loading…</p>
        ) : session.data ? (
          <Notes email={session.data.user.email} />
        ) : (
          <SignIn />
        )}
      </div>
    </main>
  )
}

function SignIn() {
  const [mode, setMode] = useState<"in" | "up">("in")
  const [error, setError] = useState("")
  const [busy, setBusy] = useState(false)

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    const form = new FormData(event.currentTarget)
    const email = String(form.get("email"))
    const password = String(form.get("password"))
    setBusy(true)
    setError("")
    const result =
      mode === "up"
        ? await authClient.signUp.email({ email, password, name: String(form.get("name")) })
        : await authClient.signIn.email({ email, password })
    setBusy(false)
    if (result.error) setError(result.error.message ?? "That didn't work. Try again.")
  }

  return (
    <form onSubmit={submit} className="flex flex-col gap-3 rounded-xl border border-stone-200 bg-white p-5 dark:border-stone-800 dark:bg-stone-900">
      <h2 className="font-medium">{mode === "up" ? "Create an account" : "Sign in"}</h2>
      {mode === "up" && <Field name="name" label="Name" autoComplete="name" />}
      <Field name="email" label="Email" type="email" autoComplete="email" />
      <Field name="password" label="Password" type="password" autoComplete={mode === "up" ? "new-password" : "current-password"} minLength={8} />
      {error && (
        <p role="alert" className="text-sm text-red-600 dark:text-red-400">
          {error}
        </p>
      )}
      <button disabled={busy} className="rounded-lg bg-stone-900 px-4 py-2 text-sm font-medium text-white disabled:opacity-50 dark:bg-stone-100 dark:text-stone-900">
        {busy ? "One moment…" : mode === "up" ? "Create account" : "Sign in"}
      </button>
      <button type="button" onClick={() => setMode(mode === "up" ? "in" : "up")} className="text-sm text-stone-500 hover:underline">
        {mode === "up" ? "Have an account? Sign in" : "New here? Create an account"}
      </button>
    </form>
  )
}

function Field(props: { name: string; label: string; type?: string; autoComplete?: string; minLength?: number }) {
  return (
    <label className="flex flex-col gap-1 text-sm">
      {props.label}
      <input
        required
        name={props.name}
        type={props.type ?? "text"}
        autoComplete={props.autoComplete}
        minLength={props.minLength}
        className="rounded-lg border border-stone-300 bg-transparent px-3 py-2 dark:border-stone-700"
      />
    </label>
  )
}

type Note = { id: string; text: string; created_at: number }

// Example feature: notes only the signed-in user can see. Replace it with the real app.
function Notes(props: { email: string }) {
  const [notes, setNotes] = useState<Note[]>([])
  const [text, setText] = useState("")

  useEffect(() => {
    void fetch("/api/notes")
      .then((response) => (response.ok ? response.json() : []))
      .then(setNotes)
  }, [])

  const add = async (event: FormEvent) => {
    event.preventDefault()
    const response = await fetch("/api/notes", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ text }),
    })
    if (!response.ok) return
    const note: Note = await response.json()
    setNotes([note, ...notes])
    setText("")
  }

  return (
    <section className="flex flex-col gap-4">
      <div className="flex items-center justify-between text-sm text-stone-500">
        <span>{props.email}</span>
        <button onClick={() => void authClient.signOut()} className="hover:underline">
          Sign out
        </button>
      </div>
      <form onSubmit={add} className="flex gap-2">
        <input
          value={text}
          onChange={(event) => setText(event.target.value)}
          placeholder="Write a note"
          aria-label="New note"
          className="min-w-0 flex-1 rounded-lg border border-stone-300 bg-white px-3 py-2 text-sm dark:border-stone-700 dark:bg-stone-900"
        />
        <button disabled={!text.trim()} className="rounded-lg bg-stone-900 px-4 py-2 text-sm font-medium text-white disabled:opacity-50 dark:bg-stone-100 dark:text-stone-900">
          Add
        </button>
      </form>
      <ul className="flex flex-col gap-2">
        {notes.map((note) => (
          <li key={note.id} className="rounded-lg border border-stone-200 bg-white px-3 py-2 text-sm dark:border-stone-800 dark:bg-stone-900">
            {note.text}
          </li>
        ))}
      </ul>
    </section>
  )
}
