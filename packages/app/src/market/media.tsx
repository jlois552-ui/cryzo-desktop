import { Button } from "@opencode/ui/button"
import { IconButton } from "@opencode/ui/icon-button"
import { Icon } from "@opencode/ui/icon"
import { Loader } from "@opencode/ui/loader"
import { For, Show } from "solid-js"
import { createStore } from "solid-js/store"
import { useLanguage } from "@/runtime/i18n/language"
import { usePlatform } from "@/runtime/platform/platform"
import { MediaThumb } from "./editor"
import type { MarketModel } from "./model"

export function MediaLibrary(props: { model: MarketModel }) {
  const language = useLanguage()
  const platform = usePlatform()
  const [state, setState] = createStore({ uploading: false })

  const upload = async () => {
    const paths = await platform.openFilePathPickerDialog?.({
      title: language.t("market.editor.upload"),
      multiple: true,
      extensions: ["png", "jpg", "jpeg", "gif", "webp", "mp4", "mov", "webm"],
    })
    if (!paths?.length) return
    setState("uploading", true)
    for (const path of paths) await props.model.run(() => props.model.api.upload(path))
    await props.model.loadMedia()
    setState("uploading", false)
  }

  return (
    <div class="flex min-h-0 min-w-0 flex-1 flex-col gap-4 px-5 py-5">
      <div class="flex items-center gap-3">
        <h2 class="text-[18px] font-medium leading-7 text-v2-text-text-base">{language.t("market.nav.media")}</h2>
        <Show when={state.uploading}>
          <Loader class="size-4" />
        </Show>
        <div class="flex-1" />
        <Button variant="contrast" icon="plus" disabled={state.uploading} onClick={() => void upload()}>
          {language.t("market.editor.upload")}
        </Button>
      </div>
      <div class="min-h-0 flex-1 overflow-y-auto" data-scrollable>
        <div class="grid grid-cols-[repeat(auto-fill,minmax(160px,1fr))] gap-3">
          <For each={props.model.state.media} fallback={<p class="col-span-full py-10 text-center text-[13px] text-v2-text-text-muted">{language.t("market.media.empty")}</p>}>
            {(item) => (
              <div class="group relative aspect-square overflow-hidden rounded-[10px] bg-v2-background-bg-layer-02">
                <MediaThumb model={props.model} url={item.url} type={item.contentType} />
                <span class="absolute inset-x-0 bottom-0 truncate bg-black/50 px-2 py-1 text-[11px] leading-4 text-white">{item.name}</span>
                <span class="absolute end-1 top-1 hidden group-hover:block">
                  <IconButton
                    variant="ghost-muted"
                    icon={<Icon name="outline-trash" />}
                    aria-label={language.t("market.media.delete", { name: item.name })}
                    onClick={async () => {
                      if ((await props.model.run(() => props.model.api.deleteMedia(item.storageId))) === undefined) return
                      await props.model.loadMedia()
                    }}
                  />
                </span>
              </div>
            )}
          </For>
        </div>
      </div>
    </div>
  )
}
