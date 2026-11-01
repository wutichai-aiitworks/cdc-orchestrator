<script setup lang="ts">
import { AlertCircle, ArrowLeft, Loader2 } from "@lucide/vue"
import { getSsoProvider, sanitizeNextPath } from "~/utils/sso"

definePageMeta({ layout: false, middleware: [] })

const route = useRoute()
const { isAuthenticated, loginWithSso, loginWithAutheliaCode } = useAuth()
const cfg = useRuntimeConfig()
const loading = ref(true)
const error = ref("")
const started = ref(false)
const ssoProvider = computed(() => getSsoProvider(cfg))
const AUTHELIA_RETRY_FLAG = "cdc_atla_callback_retried"
const EXCHANGE_TIMEOUT_MS = 12000
const PAGE_WATCHDOG_MS = 20000
let watchdog: ReturnType<typeof setTimeout> | null = null

const nextPath = computed(() => sanitizeNextPath(route.query.next))

async function consumeSsoCallback() {
  if (started.value) return
  started.value = true
  error.value = ""
  loading.value = true
  if (watchdog) clearTimeout(watchdog)
  watchdog = setTimeout(() => {
    if (!loading.value || error.value) return
    loading.value = false
    error.value = "Sign-in took too long. Go back to login and try again."
  }, PAGE_WATCHDOG_MS)

  try {
    if (isAuthenticated.value) {
      await navigateTo(nextPath.value, { replace: true })
      return
    }

    void preloadRouteComponents(nextPath.value)

    if (ssoProvider.value === "authelia") {
      const oidcError = typeof route.query.error === "string" ? route.query.error.trim() : ""
      const oidcErrorDescription =
        typeof route.query.error_description === "string" ? route.query.error_description.trim() : ""
      if (oidcError) {
        let detail = oidcErrorDescription || oidcError
        try {
          detail = decodeURIComponent(detail)
        } catch {
          // Keep raw detail if it is not URI-encoded.
        }
        throw new Error(`SSO sign-in failed: ${detail}`)
      }

      const code = typeof route.query.code === "string" ? route.query.code.trim() : ""
      const state = typeof route.query.state === "string" ? route.query.state.trim() : ""
      if (!code || !state) {
        if (typeof window !== "undefined") {
          const retried = window.sessionStorage.getItem(AUTHELIA_RETRY_FLAG) === "1"
          if (retried) {
            throw new Error("OIDC authorization response missing code/state.")
          }
          window.sessionStorage.setItem(AUTHELIA_RETRY_FLAG, "1")
        }
        const retryNext = encodeURIComponent(nextPath.value)
        await navigateTo(`/api/auth/sso/start?next=${retryNext}`, { replace: true, external: true })
        return
      }
      if (typeof window !== "undefined") {
        window.sessionStorage.removeItem(AUTHELIA_RETRY_FLAG)
      }

      const exchangeRes = await Promise.race([
        loginWithAutheliaCode(code, state),
        new Promise<never>((_, reject) =>
          setTimeout(() => reject(new Error("Authelia sign-in timed out. Please try again.")), EXCHANGE_TIMEOUT_MS),
        ),
      ])
      if (!exchangeRes.success) {
        const err = new Error(exchangeRes.error) as Error & { statusCode?: number }
        err.statusCode = exchangeRes.status
        throw err
      }
      await navigateTo(sanitizeNextPath(exchangeRes.next || nextPath.value), { replace: true })
      return
    }

    const ticket = typeof route.query.ticket === "string" ? route.query.ticket.trim() : ""
    if (!ticket) throw new Error("Missing login ticket")
    const exchangeRes = await Promise.race([
      loginWithSso(ticket),
      new Promise<never>((_, reject) =>
        setTimeout(() => reject(new Error("SSO sign-in timed out. Please try again.")), EXCHANGE_TIMEOUT_MS),
      ),
    ])
    if (!exchangeRes.success) {
      const err = new Error(exchangeRes.error) as Error & { statusCode?: number }
      err.statusCode = exchangeRes.status
      throw err
    }
    await navigateTo(sanitizeNextPath(exchangeRes.next || nextPath.value), { replace: true })
  } catch (err: any) {
    const status = err?.statusCode ?? err?.status ?? err?.response?.status
    if (status === 403) {
      await navigateTo("/auth/login?unauthorized=1", { replace: true })
      return
    }
    if (status === 401) {
      await navigateTo("/auth/login?force_login=1", { replace: true })
      return
    }
    error.value = err?.data?.message ?? err?.message ?? "SSO sign-in failed. Please try again."
  } finally {
    if (watchdog) {
      clearTimeout(watchdog)
      watchdog = null
    }
    loading.value = false
  }
}

if (import.meta.client) {
  void consumeSsoCallback()
}

onBeforeUnmount(() => {
  if (watchdog) {
    clearTimeout(watchdog)
    watchdog = null
  }
})
</script>

<template>
  <div class="relative flex min-h-[100dvh] items-center justify-center overflow-hidden bg-background p-4 animate-fade-in">
    <div class="pointer-events-none absolute inset-0 overflow-hidden">
      <div class="absolute -left-20 -top-24 h-[28rem] w-[28rem] rounded-full bg-primary/10 blur-3xl" />
      <div class="absolute -bottom-16 -right-12 h-80 w-80 rounded-full bg-primary/5 blur-3xl" />
    </div>

    <div class="relative w-full max-w-sm space-y-6 animate-slide-up">
      <div class="flex flex-col items-center gap-3">
        <img src="/favicon.png" alt="Hyper CDC" class="h-14 w-14 object-contain" />
        <p class="text-sm text-muted-foreground">Hyper CDC</p>
      </div>

      <div class="auth-card p-6 text-left">
        <h1 class="text-lg font-semibold text-foreground">Completing sign-in</h1>
        <p class="mt-1 text-sm text-muted-foreground">
          {{ ssoProvider === "authelia" ? "Authenticating with Authelia SSO" : "Authenticating with SuperTokens SSO" }}
        </p>

        <div v-if="loading" class="mt-4 flex items-center gap-2.5 text-sm text-muted-foreground">
          <Loader2 :size="16" class="shrink-0 animate-spin text-primary" />
          Please wait…
        </div>

        <div v-else-if="error" class="alert-error mt-4 flex items-start gap-2">
          <AlertCircle :size="15" class="mt-0.5 shrink-0" />
          <p>{{ error }}</p>
        </div>

        <NuxtLink
          v-if="!loading"
          to="/auth/login"
          class="mt-4 inline-flex items-center gap-1.5 text-xs text-muted-foreground transition-colors hover:text-foreground"
        >
          <ArrowLeft :size="12" />
          Back to login
        </NuxtLink>
      </div>
    </div>
  </div>
</template>
