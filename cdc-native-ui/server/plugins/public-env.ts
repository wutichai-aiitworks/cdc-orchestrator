function snake(key: string): string {
  return key.replace(/[A-Z]/g, (ch) => `_${ch}`).toUpperCase().replace(/^_/, "")
}

export default defineNitroPlugin((nitro) => {
  nitro.hooks.hook("request", (event) => {
    const pub = useRuntimeConfig(event).public as Record<string, unknown>
    for (const key of Object.keys(pub)) {
      let raw = process.env[snake(key)]
      if (key === "orchestratorUrl") {
        raw = process.env.PUBLIC_ORCHESTRATOR_URL || process.env.ORCHESTRATOR_URL || raw
      }
      if (raw === undefined || raw === "") continue
      pub[key] = typeof pub[key] === "boolean" ? raw === "true" : raw
    }
  })
})
