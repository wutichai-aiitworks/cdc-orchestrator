import monacoEditorPlugin from "vite-plugin-monaco-editor"

function env(...keys: string[]): string {
  for (const key of keys) {
    const value = process.env[key]
    if (typeof value === "string" && value.trim()) return value
  }
  return ""
}

const cookieSecure = env("SESSION_COOKIE_SECURE")
const authMode = env("AUTH_MODE").toLowerCase()

export default defineNuxtConfig({
  compatibilityDate: "2026-04-07",
  devtools: { enabled: process.env.NODE_ENV !== "production" },
  ssr: false,
  modules: ["@nuxtjs/tailwindcss"],
  components: [{ path: "~/components", extensions: ["vue"], pathPrefix: false }],
  css: ["~/assets/css/main.css"],
  runtimeConfig: {
    orchestratorUrl: env("ORCHESTRATOR_URL", "PUBLIC_ORCHESTRATOR_URL") || "http://127.0.0.1:9788",
    orchestratorToken: env("ORCHESTRATOR_TOKEN"),
    sessionCookieSecure:
      cookieSecure === "true" ? true : cookieSecure === "false" ? false : process.env.NODE_ENV === "production",
    sessionSecret: env("SESSION_SECRET"),
    ssoExchangeUrl: env("SSO_EXCHANGE_URL"),
    ssoExchangeTimeoutMs: Number(env("SSO_EXCHANGE_TIMEOUT_MS") || "12000"),
    ssoAppSecret: env("SSO_APP_SECRET"),
    autheliaTokenUrl: env("AUTHELIA_TOKEN_URL"),
    autheliaUserinfoUrl: env("AUTHELIA_USERINFO_URL"),
    autheliaClientSecret: env("AUTHELIA_CLIENT_SECRET"),
    public: {
      appName: env("APP_NAME") || "Hyper CDC WebUI",
      orchestratorUrl: env("PUBLIC_ORCHESTRATOR_URL") || "http://127.0.0.1:9788",
      authMode: authMode === "sso" ? "sso" : "none",
      ssoProvider: env("SSO_PROVIDER") || "supertokens_gateway",
      ssoAppId: env("SSO_APP_ID"),
      ssoLoginUrl: env("SSO_LOGIN_URL"),
      ssoCallbackUrl: env("SSO_CALLBACK_URL"),
      autheliaAuthorizeUrl: env("AUTHELIA_AUTHORIZE_URL"),
      autheliaClientId: env("AUTHELIA_CLIENT_ID"),
      autheliaScopes: env("AUTHELIA_SCOPES") || "openid profile email groups",
    },
  },
  app: {
    head: {
      title: "Hyper CDC WebUI",
      script: [
        {
          innerHTML:
            "(function(){var s=localStorage.getItem('ds-color-scheme');document.documentElement.classList.toggle('dark',s==='dark');})();",
          tagPosition: "head",
        },
      ],
      meta: [
        { name: "color-scheme", content: "light dark" },
        { property: "og:image", content: "/product-logo.png" },
        { name: "twitter:card", content: "summary_large_image" },
        { name: "twitter:image", content: "/product-logo.png" },
      ],
      link: [
        { rel: "icon", type: "image/x-icon", href: "/favicon.ico" },
        { rel: "icon", type: "image/png", sizes: "128x128", href: "/favicon.png" },
        { rel: "apple-touch-icon", href: "/favicon.png" },
        { rel: "preconnect", href: "https://fonts.googleapis.com" },
        { rel: "preconnect", href: "https://fonts.gstatic.com", crossorigin: "" },
        {
          rel: "stylesheet",
          href: "https://fonts.googleapis.com/css2?family=Inter:wght@300;400;500;600;700;800&display=swap",
        },
      ],
    },
  },
  vite: {
    plugins: [monacoEditorPlugin({ languageWorkers: ["json"] })],
    server: {
      allowedHosts: true,
    },
  },
})
