#!/bin/bash
# Prints the quality gate, new-code issues, uncovered new lines and overall totals
# from the last local SonarQube analysis.
#
# Usage: scripts/sonar-local-report.sh            # summary
#        ALL_ISSUES=1 scripts/sonar-local-report.sh # also list every open issue (first 500)
# Exit code: 0 when the gate passes, 1 when it fails.
set -euo pipefail

HOST="${SONAR_HOST_URL:-http://localhost:9900}"
PROJECT="${SONAR_PROJECT_KEY:-hyper-cdc-orchestrator-local}"
: "${SONAR_LOCAL_TOKEN:?export SONAR_LOCAL_TOKEN first}"

api() {
  curl -fsS -u "$SONAR_LOCAL_TOKEN:" "$HOST/api/$1"
}

echo "== Quality gate ($PROJECT)"
gate_json="$(api "qualitygates/project_status?projectKey=$PROJECT")"
echo "$gate_json" | python3 -c '
import json, sys
d = json.load(sys.stdin)["projectStatus"]
print("status:", d["status"])
for c in d["conditions"]:
    print("  %-32s %-5s actual=%s threshold=%s %s" % (
        c["metricKey"], c["status"], c.get("actualValue"), c.get("comparator"), c.get("errorThreshold")))
'

echo
echo "== New-code issues"
api "issues/search?componentKeys=$PROJECT&inNewCodePeriod=true&resolved=false&ps=500" | python3 -c '
import json, sys
d = json.load(sys.stdin)
print("total:", d["total"])
for i in d["issues"]:
    path = i["component"].split(":", 1)[-1]
    print("  %s:%s  %s  %s  %s" % (path, i.get("line", "-"), i.get("severity"), i["rule"], i["message"]))
'

echo
echo "== Files with uncovered new lines"
api "measures/component_tree?component=$PROJECT&qualifiers=FIL&metricKeys=new_lines_to_cover,new_uncovered_lines&ps=500" | python3 -c '
import json, sys
rows = []
for c in json.load(sys.stdin)["components"]:
    m = {x["metric"]: x.get("period", {}).get("value", x.get("value")) for x in c["measures"]}
    uncovered = int(float(m.get("new_uncovered_lines") or 0))
    if uncovered:
        rows.append((uncovered, int(float(m.get("new_lines_to_cover") or 0)), c["path"]))
for uncovered, total, path in sorted(rows, reverse=True):
    print("  %4d / %-4d  %s" % (uncovered, total, path))
if not rows:
    print("  none")
'

echo
echo "== Whole project"
api "measures/component?component=$PROJECT&metricKeys=ncloc,coverage,bugs,vulnerabilities,code_smells,security_hotspots,duplicated_lines_density" | python3 -c '
import json, sys
for m in json.load(sys.stdin)["component"]["measures"]:
    print("  %-26s %s" % (m["metric"], m.get("value")))
'
api "issues/search?componentKeys=$PROJECT&resolved=false&ps=1&facets=severities" | python3 -c '
import json, sys
d = json.load(sys.stdin)
sev = {v["val"]: v["count"] for f in d["facets"] for v in f["values"]}
print("  open issues: %d  %s" % (d["total"], "  ".join("%s=%d" % (k, sev.get(k, 0))
      for k in ("BLOCKER", "CRITICAL", "MAJOR", "MINOR", "INFO"))))
'

if [ "${ALL_ISSUES:-0}" = "1" ]; then
  echo
  echo "== All open issues (most severe first, max 500)"
  api "issues/search?componentKeys=$PROJECT&resolved=false&ps=500&s=SEVERITY&asc=false" | python3 -c '
import json, sys
for i in json.load(sys.stdin)["issues"]:
    path = i["component"].split(":", 1)[-1]
    print("  %-8s %s:%s  %s  %s" % (i.get("severity"), path, i.get("line", "-"), i["rule"], i["message"]))
'
fi

echo
echo "Dashboard: $HOST/dashboard?id=$PROJECT"
echo "$gate_json" | grep -q '"status":"OK"'
