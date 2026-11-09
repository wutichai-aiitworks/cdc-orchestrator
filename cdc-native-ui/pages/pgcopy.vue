<script setup lang="ts">
import { transferTester } from "~/composables/useTransferTest"
// Postgres copy (pgcopydb clone) on the shared Copy & Sync workspace.
import { ChevronRight, Copy } from "@lucide/vue"
import { applyContractOptionDefaults } from "~/composables/useMigrationTools"
import { defaultPgCopyOptions, type PgCopyConn, type PgCopyJob, type PgCopyProfile, usePgCopy } from "~/composables/usePgCopy"
import * as rules from "~/utils/transferTools/pgcopy"
import type { TransferTool } from "~/utils/transferTools/types"

definePageMeta({ layout: "default" })

const api = usePgCopy()
const contract = ref<Record<string, unknown> | null>(null)
onMounted(() => api.fetchContract().then((c) => (contract.value = c)).catch(() => {}))

const emptyConn = (): PgCopyConn => ({ type: "postgres", host: "", port: "5432", database: "", username: "", password: "", schema: "" })
const toPayload = (d: any) => ({
  name: d.name.trim(),
  source: { ...d.source, type: "postgres" },
  target: { ...d.target, type: "postgres" },
  include_tables: [...d.include_tables],
  options: Object.fromEntries(Object.entries(d.options).map(([k, v]) => [k, typeof v === "string" ? v.trim() || undefined : v])),
})
const test = transferTester("pgcopy")
const pgEnd = (role: string, c: PgCopyConn, overwrite?: string) => ({
  engine: "postgres",
  role,
  title: c.host,
  detail: c.database ? `${c.database}, port ${c.port || "5432"}` : "",
  overwrite,
})

/** ", PostgreSQL 17" once a connection test has run (the route then names the server). */
const ver = (check: any, name: string) => (check?.ok && check.raw?.server_version ? `, ${name} ${String(check.raw.server_version).split(" ")[0]}` : "")

const tool: TransferTool<PgCopyProfile, any, PgCopyJob> = {
  key: "pgcopy",
  title: "Postgres copy",
  icon: Copy,
  intro: "Copy a whole PostgreSQL database into another one in parallel, and optionally keep following its changes.",
  emptyText: "A saved copy moves schema, data and indexes from one PostgreSQL database into another with pgcopydb. Use it to seed a replica or move a database.",
  namePlaceholder: "Orders replica seed",
  noun: { one: "copy", many: "copies" },
  createLabel: "Save",
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
    include_tables: [],
    options: applyContractOptionDefaults(defaultPgCopyOptions(), contract.value ?? undefined),
  }),
  toDraft: (p) => ({
    name: p.name,
    source: { ...emptyConn(), ...p.source },
    target: { ...emptyConn(), ...p.target },
    include_tables: [...(p.include_tables || [])],
    options: { ...defaultPgCopyOptions(), ...p.options },
  }),
  toPayload,
  changeLabels: rules.changeLabels,
  verb: rules.verb,
  startedToast: rules.startedToast,
  modeLabel: (p) => (p.options.follow ? "Copy, then follow changes" : "One-time copy"),
  runResult: rules.runResult,
  steps: (p, job, log) => rules.steps(p, job, log),
  summary: rules.summary,
  effects: rules.effects,
  problems: rules.problems,
  // Runs before per-run job ids used the profile id as the job id.
  jobsOf: (jobs, id) => jobs.filter((j) => (j.profile_id || j.id) === id),
  routeSource: (p, checks) => pgEnd(`Source${ver(checks.source, "PostgreSQL")}`, p.source),
  routeDestinations: (p, checks) => [pgEnd(`Target${ver(checks.target, "PostgreSQL")}`, p.target, p.options.drop_if_exists ? "Replaces existing objects" : undefined)],
  railRoute: (p) => `${p.source.host}/${p.source.database} → ${p.target.host}/${p.target.database}`,
  railEngines: () => ["postgres", "postgres"],
  sections: (p) => [
    { id: "pc-source", label: "Source database" },
    { id: "pc-target", label: "Target database" },
    { id: "pc-scope", label: "What to copy" },
    { id: "pc-target-behaviour", label: "Target behaviour" },
    { id: "pc-speed", label: "Speed" },
    { id: "pc-follow", label: "Follow changes" },
    { id: "pc-advanced", label: "Recovery and advanced" },
  ],
  // pg_dump/pg_restore inside the image must be at least the source's major version.
  capabilityLine: (c) => {
    const pg = String(c.pg_dump || c.pgcopydb_version || "").match(/PostgreSQL\)? (\d+)/)
    return pg ? `pgcopydb with a PostgreSQL ${pg[1]} client` : ""
  },
}
const CHECK_NOTE = "Tested from the pgcopydb container, the same way a run connects."
// The API takes bytes; people think in GB.
const GB = 1024 ** 3
const gbOf = (bytes?: number | null) => (bytes ? Math.round((bytes / GB) * 100) / 100 : "")
const setGb = (options: Record<string, any>, value: string) =>
  (options.split_tables_larger_than = value.trim() === "" || Number.isNaN(Number(value)) ? undefined : Math.round(Number(value) * GB))
</script>

<template>
  <TransferWorkspace :tool="tool">
    <template #config="{ draft, checks, savedLogin, selectedId, isNew }">
      <TransferSection id="pc-source" title="Source database" why="The database to copy from. A role that can read every table is enough; following changes needs the REPLICATION privilege.">
        <TransferConnection
          :key="`src-${selectedId}`"
          v-model="draft.source"
          v-model:result="checks.source"
          engine="postgres"
          default-port="5432"
          username-placeholder="copy_reader"
          :database="{ label: 'Database', placeholder: 'shop' }"
          :check="test('source', () => ({}), () => (isNew ? undefined : selectedId))"
          :check-note="CHECK_NOTE"
          :saved-login="savedLogin('source', '5432')"
          :start-editing="isNew || !draft.source.host"
        />
      </TransferSection>

      <TransferSection id="pc-target" title="Target database" why="Where the copy goes. The role needs to create schemas, tables and indexes there.">
        <TransferConnection
          :key="`tgt-${selectedId}`"
          v-model="draft.target"
          v-model:result="checks.target"
          engine="postgres"
          default-port="5432"
          username-placeholder="copy_writer"
          :database="{ label: 'Database', placeholder: 'shop_copy' }"
          :check="test('target', () => ({}), () => (isNew ? undefined : selectedId))"
          :check-note="CHECK_NOTE"
          :saved-login="savedLogin('target', '5432')"
          :start-editing="isNew || !draft.target.host"
        />
      </TransferSection>

      <TransferSection id="pc-scope" title="What to copy" why="Leave tables empty to copy the whole database.">
        <TransferTokens v-model="draft.include_tables" label="Only these tables" help="schema.table, one per entry. Everything else is left out." placeholder="All tables" />
        <TransferSwitch v-model="draft.options.roles" label="Copy roles" help="Creates the source's roles on the target server before copying." />
        <TransferSwitch v-model="draft.options.no_role_passwords" label="Leave out role passwords" help="Roles are created without passwords; set them on the target afterwards." :disabled="!draft.options.roles" />
        <TransferSwitch v-model="draft.options.skip_large_objects" label="Leave out large objects" help="Skips pg_largeobject data. Faster when large objects are not used." />
        <TransferSwitch v-model="draft.options.skip_extensions" label="Leave out extensions" help="Extensions are not created on the target; install them yourself first." />
      </TransferSection>

      <TransferSection id="pc-target-behaviour" title="Target behaviour" why="What happens to objects that already exist in the target database.">
        <TransferSwitch
          v-model="draft.options.drop_if_exists"
          label="Drop existing objects first"
          help="Each copied object that already exists on the target is dropped and recreated. Off: the copy stops when an object already exists."
          danger
        />
        <TransferSwitch v-model="draft.options.no_owner" label="Do not set owners" help="Objects are owned by the target user. Use this when the source roles do not exist on the target." />
        <TransferSwitch v-model="draft.options.no_acl" label="Leave out privileges" help="No GRANT or REVOKE statements are restored." />
        <TransferSwitch v-model="draft.options.no_comments" label="Leave out comments" />
        <TransferSwitch v-model="draft.options.no_tablespaces" label="Ignore tablespaces" help="Everything goes into the target's default tablespace." />
        <TransferSwitch v-model="draft.options.skip_db_properties" label="Leave database settings alone" help="Settings made with ALTER DATABASE on the source are not copied." />
      </TransferSection>

      <TransferSection id="pc-speed" title="Speed" why="More parallel jobs copy faster but use more connections and CPU on both servers.">
        <div class="grid max-w-xl gap-3 sm:grid-cols-2">
          <label>
            <span class="auth-label !mb-1.5">Tables at a time</span>
            <input v-model.number="draft.options.table_jobs" type="number" min="1" max="64" class="auth-input tabular-nums" placeholder="4" />
          </label>
          <label>
            <span class="auth-label !mb-1.5">Indexes at a time</span>
            <input v-model.number="draft.options.index_jobs" type="number" min="1" max="64" class="auth-input tabular-nums" placeholder="2" />
          </label>
          <label>
            <span class="auth-label !mb-1.5">Split tables larger than</span>
            <div class="flex items-center gap-2">
              <input
                :value="gbOf(draft.options.split_tables_larger_than)"
                type="number"
                min="0"
                step="0.5"
                class="auth-input tabular-nums"
                placeholder="Never"
                @input="setGb(draft.options, ($event.target as HTMLInputElement).value)"
              />
              <span class="text-sm text-muted-foreground">GB</span>
            </div>
          </label>
          <label>
            <span class="auth-label !mb-1.5">Split into at most</span>
            <input v-model.number="draft.options.split_max_parts" type="number" min="1" class="auth-input tabular-nums" placeholder="Parts" :disabled="!draft.options.split_tables_larger_than" />
          </label>
        </div>
        <TransferSwitch v-model="draft.options.use_copy_binary" label="Copy in binary format" help="Usually faster. Both servers must have the same data types and versions of extensions." />
        <TransferSwitch v-model="draft.options.skip_vacuum" label="Skip VACUUM after loading" help="Faster, but the visibility map stays empty until autovacuum runs." />
        <TransferSwitch v-model="draft.options.skip_analyze" label="Skip ANALYZE after loading" help="Faster, but the planner has no statistics until autovacuum runs." />
      </TransferSection>

      <TransferSection id="pc-follow" title="Follow changes" why="After the copy, keep applying the source's changes to the target with logical decoding, until you stop the run.">
        <TransferSwitch v-model="draft.options.follow" label="Keep following changes" help="The run stays live after the copy. The source needs wal_level = logical." />
        <template v-if="draft.options.follow">
          <TransferSwitch v-model="draft.options.create_slot" label="Create the replication slot" help="A slot keeps WAL on the source until it is dropped. Drop it when you are done, or disk fills up." danger="Keeps WAL on the source" />
          <div class="grid max-w-xl gap-3 sm:grid-cols-2">
            <label>
              <span class="auth-label !mb-1.5">Decoding plugin</span>
              <AppSelect :model-value="draft.options.plugin || ''" aria-label="Decoding plugin" @update:model-value="draft.options.plugin = $event || undefined">
                <option value="">test_decoding (default)</option>
                <option value="wal2json">wal2json</option>
              </AppSelect>
            </label>
            <label>
              <span class="auth-label !mb-1.5">Slot name</span>
              <input v-model="draft.options.slot_name" class="auth-input font-mono !text-xs" placeholder="pgcopydb" spellcheck="false" />
            </label>
            <label>
              <span class="auth-label !mb-1.5">Replication origin</span>
              <input v-model="draft.options.origin" class="auth-input font-mono !text-xs" placeholder="pgcopydb" spellcheck="false" />
            </label>
            <label>
              <span class="auth-label !mb-1.5">Stop at LSN</span>
              <input v-model="draft.options.endpos" class="auth-input font-mono !text-xs" placeholder="Never" spellcheck="false" />
            </label>
          </div>
          <TransferSwitch v-if="draft.options.plugin === 'wal2json'" v-model="draft.options.wal2json_numeric_as_string" label="Send numbers as text" help="Keeps numeric precision with wal2json." />
        </template>
      </TransferSection>

      <TransferSection id="pc-advanced" title="Recovery and advanced" why="Rarely changed.">
        <details class="group rounded-lg border border-border">
          <summary class="flex cursor-pointer list-none items-center gap-2 px-3 py-2 text-sm font-medium text-foreground">
            <ChevronRight class="h-4 w-4 transition-transform group-open:rotate-90" :stroke-width="1.75" />
            Recovery, consistency and scope
          </summary>
          <div class="space-y-4 border-t border-border p-3">
            <TransferSwitch v-model="draft.options.fail_fast" label="Stop at the first error" help="Off: other tables continue when one fails, and the run fails at the end." />
            <TransferSwitch v-model="draft.options.resume" label="Resume the previous run" help="Continue where the last run stopped, reusing its work files." />
            <TransferSwitch v-model="draft.options.restart" label="Start over" help="Discard the previous run's work files and start from scratch." />
            <TransferSwitch v-model="draft.options.not_consistent" label="Allow a new snapshot when resuming" help="Resumes even though the data may not match a single point in time." danger="Data may be inconsistent" />
            <TransferSwitch v-model="draft.options.all_databases" label="Copy every database on the server" help="Ignores the database names and copies all of them." danger="Copies every database" />
            <TransferSwitch v-model="draft.options.skip_collations" label="Leave out collations" />
            <TransferSwitch v-model="draft.options.estimate_table_sizes" label="Estimate table sizes" help="Faster start on very large databases; sizes come from statistics instead of counting." />
            <TransferSwitch v-model="draft.options.skip_split_by_ctid" label="Do not split tables by CTID" help="Use when tables have no usable key and splitting is slow." />
            <label class="block max-w-md">
              <span class="auth-label !mb-1.5">Use snapshot</span>
              <input v-model="draft.options.snapshot" class="auth-input font-mono !text-xs" placeholder="Exported snapshot id (optional)" spellcheck="false" />
            </label>
          </div>
        </details>
      </TransferSection>
    </template>
  </TransferWorkspace>
</template>
