import { isAuthDisabled } from "~/utils/authMode"

export default defineNuxtRouteMiddleware(() => {
  if (import.meta.server) return
  const cfg = useRuntimeConfig()
  if (isAuthDisabled(cfg)) return navigateTo("/")
  const auth = useAuth()
  if (auth.ready.value && auth.isAuthenticated.value) return navigateTo("/")
})
