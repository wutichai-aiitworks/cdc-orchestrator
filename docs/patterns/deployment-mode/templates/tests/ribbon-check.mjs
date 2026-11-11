// Browser check for each mode. Start the built UI first, e.g.:
//   APP_MODE=Maintenance PORT=5173 HOST=127.0.0.1 node .output/server/index.mjs &
//   node ribbon-check.mjs Maintenance shot.png
import { chromium } from "@playwright/test"
const [label, out = "ribbon.png", url = "http://127.0.0.1:5173/"] = process.argv.slice(2)
const b = await chromium.launch()
const p = await b.newPage({ viewport: { width: 1440, height: 900 } })
const api = await (await fetch(new URL("/api/ui/mode", url))).json()
await p.goto(url, { waitUntil: "networkidle" })
const r = p.locator('[role="status"][aria-label$="mode"]')
const n = await r.count()
const bg = n ? await r.locator("div").evaluate((e) => getComputedStyle(e).backgroundColor) : "-"
const btn = await p.locator("header button").last().boundingBox()
console.log(`[${label}] api=${JSON.stringify(api)} ribbon=${n} bg=${bg} header-right-edge=${btn ? Math.round(btn.x + btn.width) : "?"}`)
// expect: production → ribbon=0; maintenance → rgb(245, 158, 11); enhanced → rgb(124, 58, 237); edge moves left only with a ribbon
await p.screenshot({ path: out, clip: { x: 1040, y: 0, width: 400, height: 200 } })
await b.close()
