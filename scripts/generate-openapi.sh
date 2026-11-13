#!/usr/bin/env bash
# Regenerate docs/openapi.json and both UI trees' public/openapi.json from docs/openapi.yaml.
# Ruby parses YAML (stdlib); Node pretty-prints so the output matches the committed format.
set -euo pipefail
cd "$(dirname "$0")/.."
json=$(ruby -ryaml -rjson -e 'print JSON.generate(YAML.load_file("docs/openapi.yaml"))' 2>/dev/null \
  | node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>process.stdout.write(JSON.stringify(JSON.parse(s),null,2)+"\n"))')
for f in docs/openapi.json cdc-native-ui/public/openapi.json; do  # kafbat-webui is frozen (2026-11-07)
  printf '%s\n' "$json" > "$f"
done
# Compact /cdc operation catalog embedded by mcp-cdc (cdc_api_catalog / cdc_api_call). The MCP
# image builds from mcp/ alone, so it gets its own copy.
printf '%s\n' "$json" | node -e '
let s=""; process.stdin.on("data",d=>s+=d).on("end",()=>{
  const spec=JSON.parse(s), out=[];
  for (const [path, ops] of Object.entries(spec.paths||{})) {
    if (!path.startsWith("/cdc/")) continue;
    for (const [m, op] of Object.entries(ops)) {
      if (!["get","post","put","patch","delete"].includes(m)) continue;
      const params=(op.parameters||[]).filter(p=>p.in==="query").map(p=>p.name);
      out.push({method:m.toUpperCase(), path, summary:op.summary||"", tag:(op.tags||[])[0]||"", query:params, write:m!=="get"});
    }
  }
  out.sort((a,b)=>a.path.localeCompare(b.path)||a.method.localeCompare(b.method));
  process.stdout.write(JSON.stringify(out,null,1)+"\n");
})' > mcp/api-catalog.json
echo "openapi.json regenerated (3 copies) + mcp/api-catalog.json"
