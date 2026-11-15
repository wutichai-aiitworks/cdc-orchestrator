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
  nitro: {
    hooks: {
      // Typed $fetch/useFetch: keep every route key (paths and methods stay checked) but declare the
      // responses as `any`. Nitro otherwise types each of ~250 routes as
      // Simplify<Serialize<Awaited<ReturnType<handler>>>>, and resolving that union on every call
      // exceeded TypeScript's instantiation depth (TS2589) whenever a route was added. Callers pass
      // the response type explicitly ($fetch<T>), so no real typing is lost.
      // Catch-all keys (`/**:path`) are left out entirely: template-literal matching against them is
      // the expensive part. Calls to those paths still work; they are just not path-checked.
      "types:extend"(types: { routes?: Record<string, Record<string, string[]>> }) {
        const routes = types.routes || {}
        for (const path of Object.keys(routes)) {
          if (path.includes("**")) {
            delete routes[path]
            continue
          }
          for (const method of Object.keys(routes[path]!)) routes[path]![method] = ["any"]
        }
      },
    },
  },
  runtimeConfig: {
    orchestratorUrl: env("ORCHESTRATOR_URL", "PUBLIC_ORCHESTRATOR_URL") || "http://127.0.0.1:9788",
    orchestratorToken: env("ORCHESTRATOR_TOKEN"),
    // mcp-cdc (optional agent surface): the MCP pages read GET /info with the orchestrator token.
    mcpUrl: env("MCP_URL") || "http://mcp-cdc:8410",
    sessionCookieSecure:
      cookieSecure === "true" ? true : cookieSecure === "false" ? false : process.env.NODE_ENV === "production",
    sessionSecret: env("SESSION_SECRET"),
    // Distinct name for the break-glass admin listener (cdc-native-ui-admin, :4321).
    sessionCookieName: env("SESSION_COOKIE_NAME") || "cdc-session",
    adminUsername: env("ADMIN_USERNAME") || "admin",
    adminPassword: env("ADMIN_PASSWORD") || "admin",
    ssoExchangeUrl: env("SSO_EXCHANGE_URL"),
    ssoExchangeTimeoutMs: Number(env("SSO_EXCHANGE_TIMEOUT_MS") || "12000"),
    ssoAppSecret: env("SSO_APP_SECRET"),
    ssoTokenUrl: env("SSO_TOKEN_URL"),
    autheliaTokenUrl: env("AUTHELIA_TOKEN_URL"),
    autheliaUserinfoUrl: env("AUTHELIA_USERINFO_URL"),
    autheliaClientSecret: env("AUTHELIA_CLIENT_SECRET"),
    public: {
      appName: env("APP_NAME") || "Hyper CDC WebUI",
      // Baked into the image by CI: `v1.2.3` for v* tags, else `<branch>_<short-sha>`.
      appVersion: env("APP_VERSION") || "dev",
      orchestratorUrl: env("PUBLIC_ORCHESTRATOR_URL") || "http://127.0.0.1:9788",
      authMode: authMode === "sso" ? "sso" : authMode === "local" ? "local" : "none",
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
