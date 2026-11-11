<script setup lang="ts">
import { AlertCircle, Eye, EyeOff, Loader2 } from "@lucide/vue"
import { isLocalAuth, isSsoAuth } from "~/utils/authMode"
import { buildSsoStartPath, getSsoProvider, sanitizeNextPath } from "~/utils/sso"

definePageMeta({ layout: false, middleware: "guest" })

const route = useRoute()
const cfg = useRuntimeConfig()
const ssoMode = computed(() => isSsoAuth(cfg))
const localMode = computed(() => isLocalAuth(cfg))
const appVersion = computed(() => String(cfg.public.appVersion || "dev"))
const { loading, loginWithPassword } = useAuth()

const username = ref("")
const password = ref("")
const showPassword = ref(false)
const loginError = ref("")
const ssoProvider = computed(() => getSsoProvider(cfg))
const ssoError = ref("")
const ssoRedirecting = ref(false)

const redirectTo = computed(() => sanitizeNextPath(route.query.next || route.query.redirect))

async function continueWithSso(forceLogin = false) {
  ssoError.value = ""
  ssoRedirecting.value = true
  const extras: Record<string, string> = forceLogin ? { force_login: "1" } : {}
  const target = buildSsoStartPath(redirectTo.value, extras)
  if (typeof window !== "undefined") {
    window.location.assign(target)
    return
  }
  await navigateTo(target, { external: true })
}

async function submitLocal() {
  loginError.value = ""
  if (!username.value.trim() || !password.value) {
    loginError.value = "Enter your username and password."
    return
  }
  const res = await loginWithPassword(username.value.trim(), password.value, redirectTo.value)
  if (!res.success) {
    loginError.value = res.error
    password.value = ""
    return
  }
  await navigateTo(sanitizeNextPath(res.next || redirectTo.value))
}

onMounted(async () => {
  if (localMode.value) return
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
        <img src="/favicon.png" alt="Hyper CDC" class="h-32 w-32 object-contain" />
        <p class="text-sm text-muted-foreground">Hyper CDC</p>
      </div>

      <form v-if="localMode" class="auth-card space-y-4 p-6" @submit.prevent="submitLocal">
        <div>
          <h1 class="text-lg font-semibold text-foreground">Sign in</h1>
          <p class="mt-1 text-sm text-muted-foreground">Use your administrator account</p>
        </div>

        <div v-if="loginError" class="alert-error flex items-start gap-2">
          <AlertCircle class="mt-0.5 h-4 w-4 shrink-0" :stroke-width="1.75" />
          <p>{{ loginError }}</p>
        </div>

        <div>
          <label for="login-username" class="auth-label">Username</label>
          <input
            id="login-username"
            v-model="username"
            type="text"
            placeholder="Username"
            class="auth-input"
            autocomplete="username"
            autocapitalize="off"
            spellcheck="false"
            autofocus
            :disabled="loading"
          />
        </div>

        <div>
          <label for="login-password" class="auth-label">Password</label>
          <div class="relative">
            <input
              id="login-password"
              v-model="password"
              :type="showPassword ? 'text' : 'password'"
              placeholder="••••••••"
              class="auth-input pr-10"
              autocomplete="current-password"
              :disabled="loading"
            />
            <button
              type="button"
              class="absolute right-3 top-1/2 -translate-y-1/2 rounded text-muted-foreground transition-colors hover:text-foreground focus:outline-none focus:ring-1 focus:ring-ring"
              :aria-label="showPassword ? 'Hide password' : 'Show password'"
              @click="showPassword = !showPassword"
            >
              <EyeOff v-if="showPassword" class="h-4 w-4" :stroke-width="1.75" />
              <Eye v-else class="h-4 w-4" :stroke-width="1.75" />
            </button>
          </div>
        </div>

        <button type="submit" class="btn-primary" :disabled="loading">
          <Loader2 v-if="loading" class="h-4 w-4 animate-spin" :stroke-width="1.75" />
          {{ loading ? "Signing in…" : "Sign in" }}
        </button>
      </form>

      <div v-else class="auth-card p-6">
        <h1 class="text-lg font-semibold text-foreground">Sign in</h1>
        <p class="mt-1 text-sm text-muted-foreground">
          {{ ssoProvider === "authelia" ? "Continue with Authelia SSO" : "Continue with SuperTokens SSO" }}
        </p>

        <div v-if="ssoError" class="alert-error mt-4 flex items-start gap-2">
          <AlertCircle class="mt-0.5 h-4 w-4 shrink-0" :stroke-width="1.75" />
          <p>{{ ssoError }}</p>
        </div>

        <button
          type="button"
          class="btn-primary mt-4"
          :disabled="ssoRedirecting"
          @click="continueWithSso(true)"
        >
          <Loader2 v-if="ssoRedirecting" class="h-4 w-4 animate-spin" :stroke-width="1.75" />
          {{ ssoRedirecting ? "Redirecting…" : "Continue with SSO" }}
        </button>
      </div>

      <p
        v-if="localMode"
        class="select-all text-center font-mono text-xs text-muted-foreground"
        :title="`Build ${appVersion}`"
      >
        {{ appVersion }}
      </p>
    </div>
  </div>
</template>
