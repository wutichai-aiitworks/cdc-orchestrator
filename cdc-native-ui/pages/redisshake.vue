<script setup lang="ts">
import { transferTester } from "~/composables/useTransferTest"
// Redis sync (RedisShake) on the shared Copy & Sync workspace. A sync runs until it is stopped.
import { ArrowRightLeft, ChevronRight } from "@lucide/vue"
import { applyContractOptionDefaults } from "~/composables/useMigrationTools"
import { defaultRedisShakeOptions, type RedisShakeConn, type RedisShakeJob, type RedisShakeProfile, useRedisShake } from "~/composables/useRedisShake"
import * as rules from "~/utils/transferTools/redisshake"
import type { StartMode, TransferTool } from "~/utils/transferTools/types"

definePageMeta({ layout: "default" })

const api = useRedisShake()
const contract = ref<Record<string, unknown> | null>(null)
onMounted(() => api.fetchContract().then((c) => (contract.value = c)).catch(() => {}))

const emptyConn = (): RedisShakeConn => ({ type: "redis", host: "", port: "6379", database: "", username: "", password: "" })
const toPayload = (d: any) => ({
  name: d.name.trim(),
  source: { ...d.source, type: "redis" },
  target: { ...d.target, type: "redis" },
  // A target database only applies to a single source database, and only when it differs.
  options: { ...d.options, source_dbs: d.options.source_dbs || [], target_db: rules.targetDb(d.options) ?? null },
})
const test = transferTester("redisshake")
const kind = (o: Record<string, any>, side: string) => [o[`${side}_cluster`] ? "Cluster" : "Single server", o[`${side}_tls`] && "TLS"].filter(Boolean).join(", ")

/** ", PostgreSQL 17" once a connection test has run (the route then names the server). */
const ver = (check: any, name: string) => (check?.ok && check.raw?.server_version ? `, ${name} ${String(check.raw.server_version).split(" ")[0]}` : "")

const tool: TransferTool<RedisShakeProfile, any, RedisShakeJob> = {
  key: "redisshake",
  title: "Redis sync",
  icon: ArrowRightLeft,
  intro: "Copy every key from one Redis to another, then keep applying changes live until you stop the sync.",
  noun: { one: "sync", many: "syncs" },
  emptyText: "A sync copies all keys from a source Redis to a target with RedisShake, then streams changes so the target stays current.",
  namePlaceholder: "Cache to new cluster",
  continuous: true,
  stopVerb: rules.stopVerb,
  api: {
    fetchProfiles: api.fetchProfiles,
    createProfile: api.createProfile,
    updateProfile: api.updateProfile,
    deleteProfile: api.deleteProfile,
    runProfile: api.runProfile,
    fetchJobs: api.fetchJobs,
    fetchJobLogs: api.fetchJobLogs,
    stopJob: api.stopJob,
    fetchCapability: api.fetchCapability,
  },
  newDraft: () => ({
    name: "",
    source: emptyConn(),
    target: emptyConn(),
    options: applyContractOptionDefaults(defaultRedisShakeOptions(), contract.value ?? undefined),
  }),
  toDraft: (p) => ({
    name: p.name,
    source: { ...emptyConn(), ...p.source },
    target: { ...emptyConn(), ...p.target },
    options: { ...defaultRedisShakeOptions(), ...p.options },
  }),
  toPayload,
  changeLabels: rules.changeLabels,
  verb: rules.verb,
  startedToast: rules.startedToast,
  modeLabel: (p) => ({ once: "One-time sync", cdc: "CDC with initial snapshot", "cdc-live": "CDC, changes only" })[rules.defaultMode(p.options)],
  // Main button = the profile's default; the others are one-off starts in the "Start as" menu.
  startModes: (p) => {
    const def = rules.defaultMode(p.options || {})
    const mk = (key: rules.StartModeKey, run: (id: string) => Promise<RedisShakeJob>) => ({
      key,
      label: rules.MODE_LABELS[key],
      help: rules.MODE_HELP[key],
      startedToast: key === "once" ? "One-time sync started" : "CDC started",
      primary: key === def,
      effects: rules.effects(p, key),
      run,
    })
    const modes: StartMode<RedisShakeJob>[] = [
      mk("cdc", (id) => api.runProfile(id, "cdc", true)),
      mk("cdc-live", (id) => api.runProfile(id, "cdc", false)),
      mk("once", (id) => api.runProfile(id, "once")),
    ]
    // A one-off "start clean": only offered when the profile does not empty the target anyway.
    if (!p.options?.empty_target) {
      modes.push({
        key: "once-empty",
        label: "Empty target, then one-time sync",
        help: `Delete ${rules.emptyScopeText(p)}, then copy every key once.`,
        startedToast: "Target emptied, one-time sync started",
        primary: false,
        effects: rules.effects(p, "once", true),
        run: (id: string) => api.runProfile(id, "once", undefined, true),
      })
    }
    return modes
  },
  runResult: rules.runResult,
  steps: (p, job, log) => rules.steps(p, job, log),
  summary: rules.summary,
  effects: rules.effects,
  problems: rules.problems,
  // Runs before per-run job ids used the profile id as the job id.
  jobsOf: (jobs, id) => jobs.filter((j) => (j.profile_id || j.id) === id),
  routeSource: (p, checks) => ({ engine: "redis", role: `Source${ver(checks.source, "Redis")}`, title: p.source.host, detail: `port ${p.source.port || "6379"}, ${kind(p.options, "source").toLowerCase()}${(p.options.source_dbs || []).length ? `, db ${p.options.source_dbs.join(", ")}` : ""}` }),
  routeDestinations: (p, checks) => [
    {
      engine: "redis",
      role: `Target${ver(checks.target, "Redis")}`,
      title: p.target.host,
      detail: `port ${p.target.port || "6379"}, ${kind(p.options, "target").toLowerCase()}${rules.targetDb(p.options) !== undefined ? `, db ${rules.targetDb(p.options)}` : ""}`,
      overwrite: (p.options.rdb_restore_command_behavior || "panic") === "rewrite" ? "Overwrites existing keys" : undefined,
    },
  ],
  railRoute: (p) => `${p.source.host}:${p.source.port || "6379"} → ${p.target.host}:${p.target.port || "6379"}`,
  railEngines: () => ["redis", "redis"],
  liveFacts: (_job, log) => {
    const s = rules.liveStats(log)
    return [
      ...(s.written !== undefined ? [{ label: "Keys written", value: s.written.toLocaleString() }] : []),
      ...(s.lag !== undefined ? [{ label: "Behind the source", value: s.lag === 0 ? "Up to date" : `${s.lag.toLocaleString()} bytes` }] : []),
    ]
  },
  sections: () => [
    { id: "rs-source", label: "Source Redis" },
    { id: "rs-target", label: "Target Redis" },
    { id: "rs-what", label: "Start mode" },
    { id: "rs-existing", label: "Existing keys" },
    { id: "rs-advanced", label: "Advanced" },
  ],
}
const CHECK_NOTE = "Tested from the redisshake container, the same way a run connects."
const START_OPTIONS = [
  { value: "cdc", label: "CDC", help: rules.MODE_HELP.cdc },
  { value: "cdc-live", label: "CDC from now", help: rules.MODE_HELP["cdc-live"] },
  { value: "once", label: "One-time sync", help: rules.MODE_HELP.once },
]
/** Default start mode → the stored RedisShake switches (full copy = sync_rdb, live changes = sync_aof). */
const setDefaultMode = (o: Record<string, any>, mode: string) => {
  o.sync_rdb = mode !== "cdc-live"
  o.sync_aof = mode !== "once"
}
const choiceModel = (o: Record<string, any>) => o.rdb_restore_command_behavior || "panic"
</script>

<template>
  <TransferWorkspace :tool="tool">
    <template #config="{ draft, checks, savedLogin, selectedId, isNew }">
      <TransferSection id="rs-source" title="Source Redis" why="The Redis to copy from. RedisShake reads it as a replica would, so the user needs the SYNC/PSYNC permissions.">
        <TransferConnection
          :key="`src-${selectedId}`"
          v-model="draft.source"
          v-model:result="checks.source"
          engine="redis"
          default-port="6379"
          host-placeholder="cache.example.internal"
          :database="null"
          username-optional
          username-placeholder="ACL user"
          :check="test('source', () => draft.options, () => (isNew ? undefined : selectedId))"
          :check-note="CHECK_NOTE"
          :summary-extra="kind(draft.options, 'source')"
          :watch-key="`${draft.options.source_cluster}|${draft.options.source_tls}`"
          :saved-login="savedLogin('source', '6379')"
          :start-editing="isNew || !draft.source.host"
        >
          <template #extra>
            <div class="grid gap-4 sm:grid-cols-2">
              <TransferSwitch v-model="draft.options.source_cluster" label="Cluster" help="The source is a Redis Cluster; any node's address works." />
              <TransferSwitch v-model="draft.options.source_tls" label="TLS" help="Connect over TLS." />
            </div>
          </template>
        </TransferConnection>
      </TransferSection>

      <TransferSection id="rs-target" title="Target Redis" why="The Redis that receives the keys. It should be empty, or you choose below what happens to keys that exist.">
        <TransferConnection
          :key="`tgt-${selectedId}`"
          v-model="draft.target"
          v-model:result="checks.target"
          engine="redis"
          default-port="6379"
          host-placeholder="cache-new.example.internal"
          :database="null"
          username-optional
          username-placeholder="ACL user"
          :check="test('target', () => draft.options, () => (isNew ? undefined : selectedId))"
          :check-note="CHECK_NOTE"
          :summary-extra="kind(draft.options, 'target')"
          :watch-key="`${draft.options.target_cluster}|${draft.options.target_tls}`"
          :saved-login="savedLogin('target', '6379')"
          :start-editing="isNew || !draft.target.host"
        >
          <template #extra>
            <div class="grid gap-4 sm:grid-cols-2">
              <TransferSwitch v-model="draft.options.target_cluster" label="Cluster" help="The target is a Redis Cluster; keys go to the right shard." />
              <TransferSwitch v-model="draft.options.target_tls" label="TLS" help="Connect over TLS." />
            </div>
          </template>
        </TransferConnection>
      </TransferSection>

      <TransferSection
        id="rs-what"
        title="Start mode"
        why="CDC keeps the target in step with the source until you stop it; a one-time sync copies once and ends. This is the main Start button; the others are always available under “Start as”."
      >
        <TransferSegmented
          :model-value="rules.defaultMode(draft.options)"
          :options="START_OPTIONS"
          label="Default start"
          @update:model-value="setDefaultMode(draft.options, $event)"
        />
        <TransferSwitch
          v-model="draft.options.empty_target"
          label="Empty the target before a full copy"
          :help="
            rules.defaultMode(draft.options) === 'cdc-live'
              ? 'Not used by CDC from now (there is no full copy). Applies when you run a one-time sync or CDC from Start as.'
              : `Deletes ${rules.emptyScopeText(draft)} before copying, so the target ends up an exact copy (no leftover keys). Without it, existing keys follow “Existing keys” below.`
          "
          danger="Deletes data on the target"
        />
        <TransferTokens
          :model-value="rules.dbTokens(draft.options)"
          label="Source databases"
          help="Database numbers to sync, for example 0 or 2. Keys keep their database number on the target. Leave empty to sync every database."
          placeholder="All databases"
          @update:model-value="draft.options.source_dbs = rules.parseDbs($event)"
        />
        <label v-if="(draft.options.source_dbs || []).length === 1" class="block max-w-xs">
          <span class="auth-label !mb-1.5">Write into target database</span>
          <input
            v-model.number="draft.options.target_db"
            type="number"
            min="0"
            max="9999"
            class="auth-input tabular-nums"
            :placeholder="`Same as source (db ${draft.options.source_dbs[0]})`"
          />
          <span class="mt-1 block text-xs text-muted-foreground">Leave empty to keep the same number.</span>
        </label>
        <p v-if="rules.dbProblem(draft.options)" class="text-xs text-amber-600 dark:text-amber-400">{{ rules.dbProblem(draft.options) }}</p>
      </TransferSection>

      <TransferSection id="rs-existing" title="Existing keys" why="What happens when a key from the source already exists on the target during the full copy.">
        <TransferSegmented
          :model-value="choiceModel(draft.options)"
          :options="rules.DUPLICATE_CHOICES.map((c) => ({ value: c.value, label: c.label, help: c.help }))"
          label="When a key already exists"
          @update:model-value="draft.options.rdb_restore_command_behavior = $event"
        />
        <p v-if="choiceModel(draft.options) === 'rewrite'" class="inline-flex items-center gap-1 text-[11px] font-medium text-amber-600 dark:text-amber-400">
          Replaces data on the target
        </p>
      </TransferSection>

      <TransferSection id="rs-advanced" title="Advanced" why="Rarely changed.">
        <details class="group rounded-lg border border-border">
          <summary class="flex cursor-pointer list-none items-center gap-2 px-3 py-2 text-sm font-medium text-foreground">
            <ChevronRight class="h-4 w-4 transition-transform group-open:rotate-90" :stroke-width="1.75" />
            Source load and write speed
          </summary>
          <div class="space-y-4 border-t border-border p-3">
            <TransferSwitch v-model="draft.options.prefer_replica" label="Read from a replica" help="In a cluster, sync from replicas to keep load off the primaries." />
            <TransferSwitch v-model="draft.options.try_diskless" label="Diskless full copy" help="Ask the source to stream the snapshot without writing it to disk." />
            <TransferSwitch v-model="draft.options.off_reply" label="Do not wait for write replies" help="Faster writes to the target, but write errors are not reported." danger="Hides write errors" />
          </div>
        </details>
      </TransferSection>
    </template>
  </TransferWorkspace>
</template>
