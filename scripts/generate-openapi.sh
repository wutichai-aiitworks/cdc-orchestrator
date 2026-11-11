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
echo "openapi.json regenerated (3 copies)"
