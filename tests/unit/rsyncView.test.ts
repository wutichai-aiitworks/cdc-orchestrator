// node --test --import ./tests/unit/register-ts.mjs tests/unit/*.test.ts
import assert from "node:assert/strict"
import { test } from "node:test"

import {
  changedFields,
  destructiveEffects,
  liveFacts,
  output,
  routeSteps,
  runProblems,
  runResult,
  runVerb,
  summarize,
  whatIsCopied,
  type JobLike,
  type ProfileLike,
} from "../../cdc-native-ui/utils/transferTools/rsync.ts"

const NOW = Date.parse("2026-11-03T10:00:00Z")
const src = { host: "web-01", port: "22", username: "deploy", auth: "key", private_key: "********", path: "/srv/www/" }
const tgt = { host: "mac-mini.lan", port: "22", username: "backup", auth: "key", private_key: "********", path: "backups/www" }
const profile = (options: Record<string, any> = {}): ProfileLike => ({ name: "Site", source: { ...src }, target: { ...tgt }, options: { transport: "auto", ...options } })
const job = (over: Partial<JobLike> = {}): JobLike => ({ id: "j1", status: "succeeded", started_at: "2026-11-03T09:59:00Z", finished_at: "2026-11-03T09:59:30Z", ...over })

test("verb and preview wording", () => {
  assert.equal(runVerb(profile()), "Run sync")
  assert.equal(runVerb(profile({ dry_run: true })), "Run preview")
  assert.equal(runResult(job({ dry_run: true, changes: { counts: { create: 3, update: 1, delete: 2 }, items: [], truncated: false } })).text, "Preview: 3 new, 1 changed and 2 deleted")
  assert.equal(runResult(job({ dry_run: true })).text, "Preview: no changes")
})

test("partial results are named by rsync exit code", () => {
  assert.equal(runResult(job({ status: "completed_with_errors", exit_code: 25 })).text, "Stopped at the delete limit")
  assert.equal(runResult(job({ status: "completed_with_errors", exit_code: 24 })).text, "Some files vanished")
  assert.equal(runResult(job({ status: "completed_with_errors", exit_code: 23 })).tone, "warn")
  assert.equal(runResult(job({ status: "running", phase: "push" })).text, "Running, pushing to the target")
})

test("auto shows one sync step until a run picks the route", () => {
  assert.deepEqual(routeSteps(profile()).map((s) => s.key), ["connect", "sync"])
  assert.deepEqual(routeSteps(profile({ transport: "relay" })).map((s) => s.key), ["connect", "pull", "push"])
  const relayed = routeSteps(profile(), job({ transport: "relay", stats: { number_of_regular_files_transferred: 4, total_transferred_file_size: 2048 } }))
  assert.deepEqual(relayed.map((s) => [s.key, s.state]), [["connect", "done"], ["pull", "done"], ["push", "done"]])
  assert.equal(relayed[2].detail, "4 files, 2 KB")
  assert.equal(relayed[0].detail, "relay route")
})

test("live and failed steps follow the phase", () => {
  const live = routeSteps(profile(), job({ status: "running", phase: "sync", transport: "direct", progress: { bytes: 1, percent: 42, rate: "1.2MB/s", eta: "0:00:10" } }), NOW)
  assert.deepEqual(live.map((s) => s.state), ["done", "now"])
  assert.equal(live[1].detail, "42%, 1.2MB/s")
  const routeCheck = routeSteps(profile(), job({ status: "running", phase: "route" }), NOW)
  assert.equal(routeCheck[0].state, "now")
  const failed = routeSteps(profile(), job({ status: "failed", phase: "connect" }))
  assert.deepEqual(failed.map((s) => s.state), ["failed", "pending"])
  const pushFailed = routeSteps(profile(), job({ status: "failed", phase: "push", transport: "relay" }))
  assert.deepEqual(pushFailed.map((s) => s.state), ["done", "done", "failed"])
})

test("summary explains the trailing-slash rule and deletion", () => {
  assert.equal(whatIsCopied("/srv/www/"), "The contents of /srv/www/")
  assert.match(whatIsCopied("/srv/www"), /itself/)
  const rows = Object.fromEntries(summarize(profile({ delete: true, max_delete: 50, excludes: ["*.log"], compress: true })).map((r) => [r.label, r.value]))
  assert.match(rows.Copies, /contents of \/srv\/www\/ on deploy@web-01/)
  assert.match(rows.Deletes, /stops after 50 deletions/)
  assert.equal(rows.Skips, "*.log")
  assert.equal(rows.Also, "compress in transit")
  assert.match(Object.fromEntries(summarize(profile()).map((r) => [r.label, r.value])).Deletes, /Nothing/)
})

test("only deleting runs are confirmed", () => {
  assert.deepEqual(destructiveEffects(profile()), [])
  assert.deepEqual(destructiveEffects(profile({ delete: true, dry_run: true })), [])
  const fx = destructiveEffects(profile({ delete: true }))
  assert.match(fx[0], /Delete every file in backups\/www on mac-mini.lan/)
  assert.match(fx[1], /no deletion limit/)
})

test("change labels track secrets and paths", () => {
  const saved = profile()
  assert.deepEqual(changedFields(saved, profile()), [])
  const draft = profile({ delete: true })
  draft.target.private_key = "-----BEGIN"
  draft.source.path = "/srv/other/"
  assert.deepEqual(changedFields(saved, draft), ["source path", "target host", "deletes"])
  assert.deepEqual(changedFields(null, draft), ["new transfer"])
})

test("problems, output and live facts", () => {
  assert.deepEqual(runProblems(job()), [])
  const p = runProblems(job({ status: "failed", error: "source: Permission denied (publickey).", hint: "check key", first_error: "x" }))
  assert.equal(p[0].fix, "check key")
  // Every rsync error is shown: the warning first, then the line that explains the skipped files.
  const multi = runProblems(job({
    status: "completed_with_errors",
    error: "rsync exit 23: some files or attributes were not transferred",
    first_error: 'rsync: [generator] recv_generator: mkdir "/opt/a2" failed: Permission denied (13)',
    errors: [
      'rsync: [generator] failed to set times on "/opt/.": Operation not permitted (1)',
      'rsync: [generator] recv_generator: mkdir "/opt/a2" failed: Permission denied (13)',
    ],
  }))
  assert.equal(multi[0].detail!.split("\n").length, 2)
  assert.match(multi[0].detail!, /mkdir "\/opt\/a2" failed: Permission denied/)
  assert.equal(output(job({ transport: "direct", stats: { number_of_regular_files_transferred: 1, number_of_deleted_files: 2 } })), "1 file, 2 deleted, direct")
  assert.equal(output(job({ status: "failed" })), "")
  const facts = liveFacts(job({ status: "running", transport: "relay", progress: { bytes: 1, percent: 5, rate: "3MB/s", eta: "", files_total: 10, files_to_check: 4 } }))
  assert.deepEqual(facts.map((f) => f.value), ["Relay", "5%", "3MB/s", "6 of 10"])
})
