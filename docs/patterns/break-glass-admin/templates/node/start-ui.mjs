// Container entrypoint. Runs the WebUI on $PORT (AUTH_MODE as configured) and, when
// ADMIN_PORT is set, a second instance of the same build on that port with
// AUTH_MODE=local — the break-glass channel for when SSO is down.
// Template from docs/patterns/break-glass-admin (reference: hyper cdc-orchestrator).
// Rename `<app>` and adjust `adminOverrides` to your env names.
//
// Supervisor rules: if either child exits, stop the other and exit with its code so
// Docker's restart policy brings both back. SIGTERM/SIGINT are forwarded.
import { spawn } from "node:child_process"
import { createInterface } from "node:readline"
import { fileURLToPath } from "node:url"

const mapEnv = fileURLToPath(new URL("./map-env.mjs", import.meta.url))
const serverEntry = process.argv[2] || "./.output/server/index.mjs"

const adminPort = String(process.env.ADMIN_PORT || "").trim()
const mainPort = String(process.env.PORT || "3000")

/**
 * Overrides for the admin instance. Each key also drops its NUXT_ / NUXT_PUBLIC_
 * copy so map-env.mjs re-derives it from the overridden value. "" means unset.
 */
const adminOverrides = {
  PORT: adminPort,
  AUTH_MODE: "local",
  // Cookies are per host, not per port: own name + own secret so a session here is
  // never valid on the main port and vice versa. Empty secret => per-process random.
  SESSION_COOKIE_NAME: "<app>-admin-session",
  SESSION_SECRET: process.env.ADMIN_SESSION_SECRET || "",
  SESSION_COOKIE_SECURE: process.env.ADMIN_SESSION_COOKIE_SECURE || "false",
  TRUST_PROXY: "false",
  APP_NAME: `${process.env.APP_NAME || "WebUI"} (admin)`,
}

function adminEnv() {
  const env = { ...process.env }
  for (const [key, value] of Object.entries(adminOverrides)) {
    delete env[`NUXT_${key}`]
    delete env[`NUXT_PUBLIC_${key}`]
    if (value) env[key] = value
    else delete env[key]
  }
  delete env.ADMIN_SESSION_SECRET
  return env
}

const children = []
let stopping = false

function pipePrefixed(stream, out, tag) {
  createInterface({ input: stream }).on("line", (line) => out.write(`[${tag}] ${line}\n`))
}

function start(tag, env) {
  const child = spawn(process.execPath, [mapEnv, serverEntry], { env, stdio: ["ignore", "pipe", "pipe"] })
  pipePrefixed(child.stdout, process.stdout, tag)
  pipePrefixed(child.stderr, process.stderr, tag)
  child.on("exit", (code, signal) => {
    if (!stopping) console.error(`[start-ui] ${tag} exited (code=${code}, signal=${signal}); stopping container`)
    shutdown(code ?? (signal ? 1 : 0))
  })
  children.push(child)
}

function shutdown(code) {
  if (stopping) return
  stopping = true
  for (const c of children) if (c.exitCode === null && c.signalCode === null) c.kill("SIGTERM")
  const force = setTimeout(() => {
    for (const c of children) if (c.exitCode === null && c.signalCode === null) c.kill("SIGKILL")
    process.exit(code)
  }, 8000)
  force.unref()
  Promise.all(children.map((c) => (c.exitCode !== null || c.signalCode !== null ? null : new Promise((r) => c.once("exit", r))))).then(
    () => process.exit(code),
  )
}

for (const sig of ["SIGTERM", "SIGINT"]) process.on(sig, () => shutdown(0))

if (adminPort && adminPort === mainPort) {
  console.error(`[start-ui] ADMIN_PORT (${adminPort}) must differ from PORT (${mainPort})`)
  process.exit(1)
}

const mainEnv = { ...process.env }
delete mainEnv.ADMIN_SESSION_SECRET
start("ui", mainEnv)
if (adminPort) {
  console.log(`[start-ui] break-glass admin listener (AUTH_MODE=local) on :${adminPort}`)
  start("admin", adminEnv())
}
