#!/usr/bin/env node
// Fail when a /cdc route in src/main.rs is missing from docs/openapi.yaml.
//   node scripts/check-openapi-routes.mjs           # report + exit 1 on gaps
//   node scripts/check-openapi-routes.mjs --list    # also print documented-only paths
import { readFileSync } from "node:fs"
import { dirname, resolve } from "node:path"
import { fileURLToPath } from "node:url"

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..")
const main = readFileSync(resolve(root, "src/main.rs"), "utf8")
const yaml = readFileSync(resolve(root, "docs/openapi.yaml"), "utf8")

// Only the `/cdc` router: routes declared between `let cdc` and the next `let `.
const cdcStart = main.search(/let\s+cdc\s*=/)
const cdcBlock = cdcStart >= 0 ? main.slice(cdcStart, main.indexOf("\n    let ", cdcStart + 10)) : main

// Copy tools share one templated block in the spec: /cdc/{tool}/...
const TOOLS = ["pgcopy", "pgloader", "redisshake", "mongocopy"]
const norm = (p) => {
  let path = p.replace(/:([A-Za-z_]+)/g, "{param}").replace(/\{[^}]+\}/g, "{param}")
  const first = path.split("/")[1]
  if (TOOLS.includes(first)) path = path.replace(`/${first}`, "/{param}")
  return path
}

const routes = new Set([...cdcBlock.matchAll(/\.route\(\s*"([^"]+)"/g)].map((m) => `/cdc${norm(m[1])}`))
const documented = new Set(
  [...yaml.matchAll(/^  (\/cdc\/[^:\s]+):\s*$/gm)].map((m) => norm(m[1].replace(/^\/cdc/, "")).replace(/^/, "/cdc"))
)

const missing = [...routes].filter((r) => !documented.has(r)).sort()
console.log(`[openapi-routes] /cdc routes: ${routes.size}, documented: ${[...routes].filter((r) => documented.has(r)).length}`)
if (process.argv.includes("--list")) {
  const extra = [...documented].filter((d) => !routes.has(d)).sort()
  if (extra.length) console.log(`documented but not routed:\n  ${extra.join("\n  ")}`)
}
if (missing.length) {
  console.error(`undocumented /cdc routes (${missing.length}):\n  ${missing.join("\n  ")}`)
  process.exit(1)
}
console.log("[openapi-routes] OK")
