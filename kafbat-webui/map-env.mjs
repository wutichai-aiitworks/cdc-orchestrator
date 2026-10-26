// Nitro private runtimeConfig keys use NUXT_<NAME>. Alias ours, then start.
for (const [key, value] of Object.entries(process.env)) {
  if (!value || key.startsWith("NUXT_")) continue
  const dest = `NUXT_${key}`
  if (!process.env[dest]) process.env[dest] = value
}
await import(new URL(process.argv[2] || "./.output/server/index.mjs", import.meta.url))
