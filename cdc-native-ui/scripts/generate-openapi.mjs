import { readFile, mkdir, writeFile } from "node:fs/promises"
import { existsSync } from "node:fs"
import { dirname, resolve } from "node:path"
import { fileURLToPath } from "node:url"
import { parse } from "yaml"

const scriptDir = dirname(fileURLToPath(import.meta.url))
const appRoot = resolve(scriptDir, "..")
const sourcePath = resolve(appRoot, "../docs/openapi.yaml")
const webOutputPath = resolve(appRoot, "public/openapi.json")
const docsOutputPath = resolve(appRoot, "../docs/openapi.json")
const checkOnly = process.argv.includes("--check")

const fail = (message) => {
  console.error(`[openapi] ${message}`)
  process.exitCode = 1
}

if (!existsSync(sourcePath)) {
  if (!checkOnly && existsSync(webOutputPath)) {
    console.log("[openapi] canonical YAML unavailable; using checked-in public/openapi.json")
  } else {
    fail(`canonical source not found: ${sourcePath}`)
  }
} else {
  const yaml = await readFile(sourcePath, "utf8")
  const document = parse(yaml)

  if (!document || document.openapi !== "3.0.3" || !document.info || !document.paths) {
    fail("canonical YAML is not a valid Hyper CDC OpenAPI 3.0.3 document")
  } else {
    const output = `${JSON.stringify(document, null, 2)}\n`
    const targets = [webOutputPath, docsOutputPath]

    for (const target of targets) {
      if (checkOnly) {
        if (!existsSync(target) || (await readFile(target, "utf8")) !== output) {
          fail(`${target} is stale; run npm run openapi:generate`)
        }
      } else {
        await mkdir(dirname(target), { recursive: true })
        await writeFile(target, output, "utf8")
        console.log(`[openapi] wrote ${target}`)
      }
    }
  }
}
