<script setup lang="ts">
import { transferTester } from "~/composables/useTransferTest"
// Load to Postgres (pgloader) on the shared Copy & Sync workspace.
import { Database } from "@lucide/vue"
import { applyContractOptionDefaults } from "~/composables/useMigrationTools"
import { defaultPgLoaderOptions, type PgLoaderConn, type PgLoaderJob, type PgLoaderProfile, usePgLoader } from "~/composables/usePgLoader"
import * as rules from "~/utils/transferTools/pgloader"
import type { TransferTool } from "~/utils/transferTools/types"

definePageMeta({ layout: "default" })

const api = usePgLoader()
const contract = ref<Record<string, unknown> | null>(null)
onMounted(() => api.fetchContract().then((c) => (contract.value = c)).catch(() => {}))

const emptySource = (): PgLoaderConn => ({ type: "mysql", host: "", port: "3306", database: "", username: "", password: "" })
const emptyTarget = (): PgLoaderConn => ({ type: "postgres", host: "", port: "5432", database: "", username: "", password: "" })
const num = (v: unknown) => (v === "" || v == null || Number.isNaN(Number(v)) ? undefined : Number(v))
const toPayload = (d: any) => {
  const file = rules.isFile(d)
  return {
    name: d.name.trim(),
    // A file source is just a path (kept in `database`); no host or login is sent.
    source: file ? { type: d.source.type, host: "", port: "", database: d.source.database.trim(), username: "", password: "" } : { ...d.source },
    target: { ...d.target, type: "postgres" },
    include_tables: rules.isCsv(d) ? [] : [...d.include_tables],
    options: {
      ...d.options,
      workers: num(d.options.workers),
      concurrency: num(d.options.concurrency),
      csv_skip_header: num(d.options.csv_skip_header),
      target_table: d.options.target_table?.trim() || undefined,
    },
  }
}
const test = transferTester("pgloader")

/** ", PostgreSQL 17" once a connection test has run (the route then names the server). */
const ver = (check: any, name: string) => (check?.ok && check.raw?.server_version ? `, ${name} ${String(check.raw.server_version).split(" ")[0]}` : "")

const tool: TransferTool<PgLoaderProfile, any, PgLoaderJob> = {
  key: "pgloader",
  title: "Load to Postgres",
  icon: Database,
  intro: "Load a MySQL, MariaDB, SQL Server or SQLite database, or a CSV file, into PostgreSQL. Types are converted for you.",
  noun: { one: "load", many: "loads" },
  emptyText: "A load brings another database or a CSV file into PostgreSQL with pgloader, creating the tables and converting types.",
  namePlaceholder: "Legacy shop to Postgres",
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
    source: emptySource(),
    target: emptyTarget(),
    include_tables: [],
    options: applyContractOptionDefaults(defaultPgLoaderOptions(), contract.value ?? undefined),
  }),
  toDraft: (p) => ({
    name: p.name,
    source: { ...emptySource(), ...p.source },
    target: { ...emptyTarget(), ...p.target },
    include_tables: [...(p.include_tables || [])],
    options: { ...defaultPgLoaderOptions(), ...p.options },
  }),
  toPayload,
  changeLabels: rules.changeLabels,
  verb: rules.verb,
  startedToast: rules.startedToast,
  modeLabel: (p) => `${rules.engineOf(p.source.type).label} to PostgreSQL`,
  runResult: rules.runResult,
  steps: (p, job, log) => rules.steps(p, job, log),
  summary: rules.summary,
  effects: rules.effects,
  problems: rules.problems,
  // Runs before per-run job ids used the profile id as the job id.
  jobsOf: (jobs, id) => jobs.filter((j) => (j.profile_id || j.id) === id),
  routeSource: (p) =>
    rules.isFile(p)
      ? { engine: p.source.type, role: rules.engineOf(p.source.type).label, title: p.source.database.split("/").pop() || "", detail: p.source.database, mono: true }
      : { engine: p.source.type, role: `Source, ${rules.engineOf(p.source.type).label}`, title: p.source.host, detail: p.source.database ? `${p.source.database}, port ${p.source.port || rules.engineOf(p.source.type).port}` : "" },
  routeDestinations: (p, checks) => [
    {
      engine: "postgres",
      role: `Target${ver(checks.target, "PostgreSQL") || ", PostgreSQL"}`,
      title: p.target.host,
      detail: rules.isCsv(p) ? `${p.target.database}, table ${p.options.target_table || "?"}` : p.target.database ? `${p.target.database}, port ${p.target.port || "5432"}` : "",
      overwrite: !rules.isCsv(p) && p.options.drop_if_exists ? "Replaces existing tables" : !rules.isCsv(p) && p.options.truncate ? "Empties tables first" : undefined,
    },
  ],
  railRoute: (p) => `${rules.isFile(p) ? p.source.database.split("/").pop() : `${p.source.host}/${p.source.database}`} → ${p.target.host}/${p.target.database}`,
  railEngines: (p) => [p.source.type, "postgres"],
  sections: (p) =>
    [
      { id: "pl-source", label: "Source" },
      { id: "pl-target", label: "Target database" },
      { id: "pl-scope", label: rules.isCsv(p) ? "CSV format" : "What to load" },
      !rules.isCsv(p) && { id: "pl-behaviour", label: "Target behaviour" },
      !rules.isCsv(p) && { id: "pl-speed", label: "Speed" },
    ].filter(Boolean) as { id: string; label: string }[],
  capabilityLine: (c) => {
    const v = String(c.pgloader_version || "").match(/version "?([^"\s]+)"?/)
    return v ? `pgloader ${v[1]}` : ""
  },
}

const ENGINE_OPTIONS = rules.ENGINES.map((e) => ({ value: e.value, label: e.label }))
const setEngine = (draft: any, value: string) => {
  const meta = rules.engineOf(value)
  const wasFile = rules.isFile(draft)
  draft.source.type = value
  draft.source.port = meta.port
  if (meta.file !== wasFile) draft.source.database = ""
}
const CHECK_NOTE = "Tested from the pgloader container, the same way a run connects."
</script>

<template>
  <TransferWorkspace :tool="tool">
    <template #config="{ draft, checks, savedLogin, selectedId, isNew }">
      <TransferSection id="pl-source" title="Source" :why="rules.isFile(draft) ? 'The file must be readable inside the pgloader container, for example on a mounted volume.' : 'The database to load from. A read-only login is enough.'">
        <div>
          <span class="auth-label !mb-1.5">Load from</span>
          <TransferSegmented :model-value="draft.source.type" :options="ENGINE_OPTIONS" label="Source engine" @update:model-value="setEngine(draft, $event)" />
        </div>
        <TransferConnection
          :key="`src-${selectedId}-${rules.isFile(draft)}`"
          v-model="draft.source"
          v-model:result="checks.source"
          :engine="draft.source.type"
          :default-port="rules.engineOf(draft.source.type).port"
          :database="{ label: 'Database', placeholder: 'shop' }"
          :file="rules.isFile(draft) ? { label: draft.source.type === 'csv' ? 'CSV file path' : 'SQLite file path', placeholder: draft.source.type === 'csv' ? '/data/orders.csv' : '/data/shop.sqlite' } : null"
          :check="test('source', () => ({ csv_delimiter: draft.options.csv_delimiter }), () => (isNew ? undefined : selectedId))"
          :check-note="CHECK_NOTE"
          :saved-login="savedLogin('source', rules.engineOf(draft.source.type).port)"
          :start-editing="isNew || (!draft.source.host && !draft.source.database)"
        />
      </TransferSection>

      <TransferSection id="pl-target" title="Target database" why="The PostgreSQL database to load into. The role needs to create tables there.">
        <TransferConnection
          :key="`tgt-${selectedId}`"
          v-model="draft.target"
          v-model:result="checks.target"
          engine="postgres"
          default-port="5432"
          username-placeholder="loader"
          :database="{ label: 'Database', placeholder: 'shop' }"
          :check="test('target', () => ({}), () => (isNew ? undefined : selectedId))"
          :check-note="CHECK_NOTE"
          :saved-login="savedLogin('target', '5432')"
          :start-editing="isNew || !draft.target.host"
        />
      </TransferSection>

      <TransferSection v-if="rules.isCsv(draft)" id="pl-scope" title="CSV format" why="How the file is read, and which table its rows go into. Rows are added to the table; existing rows stay.">
        <div class="grid max-w-xl gap-3 sm:grid-cols-3">
          <label class="sm:col-span-3">
            <span class="auth-label !mb-1.5">Into table</span>
            <input v-model="draft.options.target_table" class="auth-input font-mono !text-xs" placeholder="public.orders" spellcheck="false" />
          </label>
          <label>
            <span class="auth-label !mb-1.5">Field separator</span>
            <input v-model="draft.options.csv_delimiter" class="auth-input font-mono" maxlength="1" placeholder="," />
          </label>
          <label>
            <span class="auth-label !mb-1.5">Header lines to skip</span>
            <input v-model.number="draft.options.csv_skip_header" type="number" min="0" class="auth-input tabular-nums" placeholder="1" />
          </label>
        </div>
      </TransferSection>

      <template v-else>
        <TransferSection id="pl-scope" title="What to load" why="Leave empty to load every table.">
          <TransferTokens v-model="draft.include_tables" label="Only tables matching" help="Table names or patterns, as pgloader matches them." placeholder="All tables" />
        </TransferSection>

        <TransferSection id="pl-behaviour" title="Target behaviour" why="What pgloader does in the target database before and after loading the rows.">
          <TransferSwitch v-model="draft.options.create_tables" label="Create the tables" help="Off: the tables must already exist with matching columns." />
          <TransferSwitch
            v-model="draft.options.drop_if_exists"
            label="Drop existing tables first"
            help="Tables being loaded that already exist are dropped and recreated."
            :disabled="!draft.options.create_tables"
            danger
          />
          <TransferSwitch v-model="draft.options.truncate" label="Empty tables before loading" help="Existing rows are removed (TRUNCATE) before the new rows are loaded." danger />
          <TransferSwitch v-model="draft.options.create_indexes" label="Build indexes" help="Off: faster load; create the indexes yourself afterwards." />
          <TransferSwitch v-model="draft.options.reset_sequences" label="Reset sequences" help="Sequences continue after the highest loaded value, so new rows get fresh ids." />
        </TransferSection>

        <TransferSection id="pl-speed" title="Speed" why="More workers load faster but use more memory and connections.">
          <div class="grid max-w-xl gap-3 sm:grid-cols-2">
            <label>
              <span class="auth-label !mb-1.5">Workers</span>
              <input v-model.number="draft.options.workers" type="number" min="1" max="64" class="auth-input tabular-nums" placeholder="Default" />
            </label>
            <label>
              <span class="auth-label !mb-1.5">Concurrent loads</span>
              <input v-model.number="draft.options.concurrency" type="number" min="1" max="64" class="auth-input tabular-nums" placeholder="Default" />
            </label>
          </div>
        </TransferSection>
      </template>
    </template>
  </TransferWorkspace>
</template>
