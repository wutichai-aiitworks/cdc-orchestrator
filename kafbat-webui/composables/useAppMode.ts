import { APP_MODE_RIBBON, type AppMode } from "~/utils/appMode"

/** APP_MODE for the whole app (loaded once in app.vue, during server rendering). */
export const useAppMode = () => {
  const mode = useState<AppMode>("app-mode", () => "production")
  return { mode, ribbon: computed(() => APP_MODE_RIBBON[mode.value] ?? null) }
}
