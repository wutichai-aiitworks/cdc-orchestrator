<script setup lang="ts">
import { AlertCircle, Loader2 } from "@lucide/vue"
import { isSsoAuth } from "~/utils/authMode"
import { buildSsoStartPath, getSsoProvider, sanitizeNextPath } from "~/utils/sso"

definePageMeta({ layout: false, middleware: "guest" })

const route = useRoute()
const cfg = useRuntimeConfig()
const ssoMode = computed(() => isSsoAuth(cfg))
const ssoProvider = computed(() => getSsoProvider(cfg))
const ssoError = ref("")
const ssoRedirecting = ref(false)

const redirectTo = computed(() => sanitizeNextPath(route.query.next || route.query.redirect))

async function continueWithSso(forceLogin = false) {
  ssoError.value = ""
  ssoRedirecting.value = true
  const extras = forceLogin ? { force_login: "1" } : {}
  const target = buildSsoStartPath(redirectTo.value, extras)
  if (typeof window !== "undefined") {
    window.location.assign(target)
    return
  }
  await navigateTo(target, { external: true })
}

onMounted(async () => {
  if (!ssoMode.value) {
    await navigateTo("/")
    return
  }
  const unauthorized = route.query.unauthorized
  if (
    unauthorized === "1" ||
    unauthorized === "true" ||
    route.query.error === "unauthorized" ||
    route.query.error_code === "403"
  ) {
    ssoError.value = "Access denied — you do not have permission to use this application."
    return
  }
  const forceLogin =
    route.query.force_login === "1" ||
    route.query.force_login === "true" ||
    route.query.switch_account === "1" ||
    route.query.switch_account === "true"
  await continueWithSso(forceLogin)
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

      <div class="auth-card p-6">
        <h1 class="text-lg font-semibold text-foreground">Sign in</h1>
        <p class="mt-1 text-sm text-muted-foreground">
          {{ ssoProvider === "authelia" ? "Continue with Authelia SSO" : "Continue with SuperTokens SSO" }}
        </p>

        <div v-if="ssoError" class="alert-error mt-4 flex items-start gap-2">
          <AlertCircle :size="15" class="mt-0.5 shrink-0" />
          <p>{{ ssoError }}</p>
        </div>

        <button
          type="button"
          class="btn-primary mt-4"
          :disabled="ssoRedirecting"
          @click="continueWithSso(true)"
        >
          <Loader2 v-if="ssoRedirecting" :size="16" class="animate-spin" />
          {{ ssoRedirecting ? "Redirecting…" : "Continue with SSO" }}
        </button>
      </div>
    </div>
  </div>
</template>
