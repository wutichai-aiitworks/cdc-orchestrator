// Nitro runtimeConfig: private keys use NUXT_<NAME>, public keys use NUXT_PUBLIC_<NAME>.
for (const [key, value] of Object.entries(process.env)) {
  if (!value || key.startsWith("NUXT_")) continue
  const dest = `NUXT_${key}`
  if (!process.env[dest]) process.env[dest] = value
}

const publicAliases = {
  APP_NAME: "NUXT_PUBLIC_APP_NAME",
  PUBLIC_ORCHESTRATOR_URL: "NUXT_PUBLIC_ORCHESTRATOR_URL",
  AUTH_MODE: "NUXT_PUBLIC_AUTH_MODE",
  SSO_PROVIDER: "NUXT_PUBLIC_SSO_PROVIDER",
  SSO_APP_ID: "NUXT_PUBLIC_SSO_APP_ID",
  SSO_LOGIN_URL: "NUXT_PUBLIC_SSO_LOGIN_URL",
  SSO_CALLBACK_URL: "NUXT_PUBLIC_SSO_CALLBACK_URL",
  AUTHELIA_AUTHORIZE_URL: "NUXT_PUBLIC_AUTHELIA_AUTHORIZE_URL",
  AUTHELIA_CLIENT_ID: "NUXT_PUBLIC_AUTHELIA_CLIENT_ID",
  AUTHELIA_SCOPES: "NUXT_PUBLIC_AUTHELIA_SCOPES",
}
for (const [src, dest] of Object.entries(publicAliases)) {
  const value = process.env[src]
  if (value && !process.env[dest]) process.env[dest] = value
}

await import(new URL(process.argv[2] || "./.output/server/index.mjs", import.meta.url))
