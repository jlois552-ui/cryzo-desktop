import { createContext, useContext, type ParentProps } from "solid-js"
import { useNavigate, useParams } from "@solidjs/router"

/** Route adapters and embedded chats share identity and durable state, but own their navigation. */
type SessionSurface = {
  params: { readonly id?: string; readonly serverKey?: string }
  draftId: () => string | undefined
  navigate: ReturnType<typeof useNavigate>
  started: (id: string) => void
  newChat: () => Promise<void>
  context: () => string
}

const Context = createContext<SessionSurface>()

export function SessionSurfaceProvider(props: ParentProps<{ value: SessionSurface }>) {
  return <Context.Provider value={props.value}>{props.children}</Context.Provider>
}

export function useSessionSurface() {
  return useContext(Context)
}

export function useSessionParams<T extends { id?: string; serverKey?: string }>() {
  const surface = useSessionSurface()
  const route = useParams<T>()
  return (surface?.params ?? route) as T
}

export function useSessionNavigate() {
  const surface = useSessionSurface()
  const navigate = useNavigate()
  return surface?.navigate ?? navigate
}
