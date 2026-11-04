<script setup lang="ts">
// Postgres dump on the shared Copy & Sync workspace. Extras over the other tools: a real
// connection test from the sidecar (with a schema picker), SFTP destinations, and the dump file.
import { ChevronRight, HardDriveDownload, Plus } from "@lucide/vue"
import {
  defaultPgDumpOptions,
  emptyDestination,
  pgDumpFileUrl,
  type PgDumpConn,
  type PgDumpJob,
  type PgDumpProfile,
  usePgDump,
} from "~/composables/usePgDump"
import { formatBytes } from "~/utils/bytes"
import {
  destructiveEffects,
  isRestoreMode,
  pgDumpChangeLabels,
  routeSteps,
  runProblems,
  runResult,
  runVerb,
  startedToast,
  summarize,
} from "~/utils/pgDumpView"
import type { ConnCheck, TransferTool } from "~/utils/transferTools/types"

definePageMeta({ layout: "default" })

const api = usePgDump()
const toast = useToast()
const emptyConn = (): PgDumpConn => ({ type: "postgres", host: "", port: "5432", database: "", username: "", password: "", schema: "" })
const num = (v: unknown) => (v === null || v === undefined || v === "" || Number.isNaN(Number(v)) ? null : Number(v))

const toPayload = (d: any) => {
  const restore = d.mode === "dump_restore"
  return {
    name: d.name.trim(),
    mode: d.mode,
    source: { ...d.source, type: "postgres" },
    target: restore ? { ...d.target, type: "postgres" } : null,
    include_tables: [...d.include_tables],
    options: {
      ...d.options,
      format: restore ? "custom" : d.options.format,
      compress: num(d.options.compress),
      restore_jobs: num(d.options.restore_jobs),
      lock_wait_timeout_ms: num(d.options.lock_wait_timeout_ms),
      restore_if_exists: !!(d.options.restore_clean && d.options.restore_if_exists),
    },
    destinations: d.destinations.map((x: any) => ({
      ...x,
      name: x.name.trim() || x.host.trim(),
      host: x.host.trim(),
      username: x.username.trim(),
      remote_dir: x.remote_dir.trim(),
      host_key_fingerprint: x.host_key_fingerprint.trim(),
      password: x.auth === "password" ? x.password : "",
      private_key: x.auth === "key" ? x.private_key : "",
      passphrase: x.auth === "key" ? x.passphrase : "",
    })),
  }
}

/** Real test from the sidecar: proves the network path and login a dump will use. */
const tester = (side: "source" | "target", sslmode: () => string | undefined, profileId: () => string | undefined) => async (c: any): Promise<Omit<ConnCheck, "at">> => {
  const r = await api.testConnection({ connection: { ...c, type: "postgres" }, sslmode: sslmode() || undefined, profile_id: profileId(), side })
  if (!r.ok) return { ok: false, error: r.error, hint: r.hint }
  const n = r.schemas?.length ?? 0
  return {
    ok: true,
    facts: [`PostgreSQL ${r.server_version?.split(" ")[0]}`, formatBytes(r.database_bytes), n === 1 ? "1 schema" : `${n} schemas`, `${r.latency_ms} ms`, ...(r.ssl ? ["TLS on"] : [])],
    warning: r.compatible === false ? r.hint || undefined : undefined,
    raw: r,
  }
}

const pgEnd = (role: string, c: PgDumpConn, check: ConnCheck | null, overwrite?: string) => ({
  engine: "postgres",
  role: check?.ok && check.raw?.server_major ? `${role}, PostgreSQL ${check.raw.server_major}` : role,
  title: c.host,
  detail: c.database ? `${c.database}, port ${c.port || "5432"}` : "",
  overwrite,
})
const storedOn = (j: PgDumpJob) => {
  const up = (j.uploads || []).filter((u) => u.status === "uploaded").map((u) => u.name)
  if (j.download_path) up.push("dump volume")
  return up.join(", ")
}

const tool: TransferTool<PgDumpProfile, any, PgDumpJob> = {
  key: "pgdump",
  title: "Postgres dump",
  icon: HardDriveDownload,
  intro: "Back up a PostgreSQL database to a file, copy it to SFTP servers, or restore it into another database.",
  emptyText: "A transfer remembers which database to dump, what to include, and where the file goes. Run it whenever you need a fresh copy.",
  namePlaceholder: "Shop nightly backup",
  api: {
    fetchProfiles: api.fetchProfiles,
    createProfile: api.createProfile,
    updateProfile: api.updateProfile,
    deleteProfile: api.deleteProfile,
    runProfile: api.runProfile,
    fetchJobs: api.fetchJobs,
    fetchJobLogs: api.fetchJobLogs,
    stopJob: api.stopJob,
    deleteJob: api.deleteJob,
    fetchCapability: api.fetchCapability,
  },
  newDraft: () => ({ name: "", mode: "dump", source: emptyConn(), target: emptyConn(), include_tables: [], options: defaultPgDumpOptions(), destinations: [] }),
  toDraft: (p) => ({
    name: p.name,
    mode: p.mode || "dump",
    source: { ...emptyConn(), ...p.source },
    target: { ...emptyConn(), ...(p.target || {}) },
    include_tables: [...(p.include_tables || [])],
    options: { ...defaultPgDumpOptions(), ...p.options },
    destinations: (p.destinations || []).map((d) => ({ ...emptyDestination(), ...d })),
  }),
  toPayload,
  changeLabels: pgDumpChangeLabels,
  verb: runVerb,
  startedToast,
  modeLabel: (p) => (isRestoreMode(p) ? "Dump and restore" : p.destinations.length ? "Dump to SFTP" : "Dump to file"),
  runResult,
  steps: (p, job, _log, now) => routeSteps(p, job, now),
  summary: summarize,
  effects: destructiveEffects,
  problems: (job) => runProblems(job),
  jobsOf: (jobs, id) => jobs.filter((j) => j.profile_id === id),
  routeSource: (p, checks) => pgEnd("Source", p.source, checks.source),
  routeDestinations: (p, checks, cap) => {
    if (isRestoreMode(p)) return [pgEnd("Target", p.target, checks.target, p.options.restore_clean ? "Replaces existing objects" : undefined)]
    const ends: any[] = p.destinations.map((d: any) => ({ engine: "sftp", role: "SFTP", title: d.name || d.host, detail: d.remote_dir || "home directory", mono: true }))
    if (!ends.length || p.options.keep_local)
      ends.push({ engine: "file", role: "File", title: "Dump volume", detail: `kept ${Number(cap?.retention_hours) || 72} h for download` })
    return ends
  },
  railRoute: (p) =>
    `${p.source.host}/${p.source.database} → ${p.mode === "dump_restore" && p.target ? `${p.target.host}/${p.target.database}` : p.destinations?.length ? `${p.destinations.length} SFTP` : "file"}`,
  railEngines: (p) => ["postgres", p.mode === "dump_restore" ? "postgres" : p.destinations?.length ? "sftp" : undefined],
  sections: (p) =>
    [
      { id: "pd-source", label: "Source database" },
      isRestoreMode(p) && { id: "pd-target", label: "Target database" },
      { id: "pd-scope", label: "What to dump" },
      !isRestoreMode(p) && { id: "pd-file", label: "File" },
      isRestoreMode(p) && { id: "pd-restore", label: "Restore behaviour" },
      { id: "pd-sftp", label: "Store on SFTP servers" },
      { id: "pd-advanced", label: "Advanced" },
    ].filter(Boolean) as { id: string; label: string }[],
  capabilityLine: (c) => {
    const v = String(c.pg_dump || "").replace(/^pg_dump \(PostgreSQL\)\s*/, "").split(" ")[0]
    const free = Number(c.free_mb)
    return [v && `pg_dump ${v}`, Number.isFinite(free) && free >= 0 && `${formatBytes(free * 1024 * 1024)} free on the dump volume`].filter(Boolean).join(" · ")
  },
  liveFacts: (j) => [{ label: "Tables dumped", value: String(j.tables_dumped ?? 0) }],
  output: (j) => (j.artifact ? `${formatBytes(j.artifact.bytes)} ${j.artifact.format}${storedOn(j) ? `, on ${storedOn(j)}` : ""}` : ""),
}
// ── configuration helpers (operate on the slot's draft) ──────────────────
const MODE_OPTIONS = [
  { value: "dump", label: "Keep as a file", help: "The file stays on the dump volume for download, and can be copied to SFTP servers." },
  { value: "dump_restore", label: "Restore into a database", help: "The dump is restored into a target database with pg_restore. The archive is kept as well." },
]
const contentsOf = (o: any) => (o.schema_only ? "schema" : o.data_only ? "data" : "all")
const setContents = (o: any, v: string) => {
  o.schema_only = v === "schema"
  o.data_only = v === "data"
}
const toggleSchema = (o: any, name: string) => {
  const s: string[] = o.schemas || []
  o.schemas = s.includes(name) ? s.filter((x) => x !== name) : [...s, name]
}
const lockSeconds = (o: any) => (o.lock_wait_timeout_ms ? Math.round(o.lock_wait_timeout_ms / 1000) : "")
const setLockSeconds = (o: any, v: string) => (o.lock_wait_timeout_ms = v === "" ? null : Math.max(1, Number(v)) * 1000)
const advancedCount = (o: any) =>
  ["no_owner", "no_privileges", "no_comments", "no_publications", "no_subscriptions", "serializable_deferrable"].filter((k) => o[k]).length + (o.lock_wait_timeout_ms ? 1 : 0)
const exclusionsCount = (o: any) => (o.exclude_schemas?.length || 0) + (o.exclude_tables?.length || 0) + (o.exclude_table_data?.length || 0)
const pin = (draft: any, name: string, fingerprint: string) => {
  const d = draft.destinations.find((x: any) => (x.name || x.host) === name)
  if (!d) return
  d.host_key_fingerprint = fingerprint
  toast.success(`Key for ${name} pinned. Save to keep it.`)
}
</script>

<template>
  <TransferWorkspace :tool="tool">
    <template #general="{ draft }">
      <div>
        <span class="auth-label !mb-1.5">What happens to the dump</span>
        <TransferSegmented v-model="draft.mode" label="Mode" :options="MODE_OPTIONS" />
      </div>
    </template>

    <template #config="{ draft, checks, savedLogin, selectedId, isNew }">
      <TransferSection id="pd-source" title="Source database" why="The database to dump. A read-only role is enough, for example one granted pg_read_all_data.">
        <TransferConnection
          :key="`src-${selectedId}`"
          v-model="draft.source"
          v-model:result="checks.source"
          engine="postgres"
          default-port="5432"
          username-placeholder="backup_ro"
          :database="{ label: 'Database', placeholder: 'shop' }"
          :check="tester('source', () => draft.options.source_sslmode, () => (isNew ? undefined : selectedId))"
          check-note="Tested from the dump service, the same way a run connects."
          :summary-extra="`TLS ${draft.options.source_sslmode || 'server default'}`"
          :watch-key="draft.options.source_sslmode || ''"
          :saved-login="savedLogin('source', '5432')"
          :start-editing="isNew || !draft.source.host"
        >
          <template #extra>
            <label class="block max-w-xs">
              <span class="auth-label !mb-1.5">TLS</span>
              <AppSelect :model-value="draft.options.source_sslmode || ''" aria-label="TLS mode" @update:model-value="draft.options.source_sslmode = $event || undefined">
                <option value="">Server default (prefer)</option>
                <option value="disable">Off</option>
                <option value="require">Required</option>
                <option value="verify-ca">Verify CA</option>
                <option value="verify-full">Verify CA and host name</option>
              </AppSelect>
            </label>
          </template>
        </TransferConnection>
      </TransferSection>

      <TransferSection v-if="draft.mode === 'dump_restore'" id="pd-target" title="Target database" why="Where the dump is restored. The role needs to create objects in this database.">
        <TransferConnection
          :key="`tgt-${selectedId}`"
          v-model="draft.target"
          v-model:result="checks.target"
          engine="postgres"
          default-port="5432"
          username-placeholder="restore_user"
          :database="{ label: 'Database', placeholder: 'shop_copy' }"
          :check="tester('target', () => draft.options.target_sslmode, () => (isNew ? undefined : selectedId))"
          check-note="Tested from the dump service, the same way a run connects."
          :summary-extra="`TLS ${draft.options.target_sslmode || 'server default'}`"
          :watch-key="draft.options.target_sslmode || ''"
          :saved-login="savedLogin('target', '5432')"
          :start-editing="isNew || !draft.target.host"
        >
          <template #extra>
            <label class="block max-w-xs">
              <span class="auth-label !mb-1.5">TLS</span>
              <AppSelect :model-value="draft.options.target_sslmode || ''" aria-label="TLS mode" @update:model-value="draft.options.target_sslmode = $event || undefined">
                <option value="">Server default (prefer)</option>
                <option value="disable">Off</option>
                <option value="require">Required</option>
                <option value="verify-ca">Verify CA</option>
                <option value="verify-full">Verify CA and host name</option>
              </AppSelect>
            </label>
          </template>
        </TransferConnection>
      </TransferSection>

      <TransferSection id="pd-scope" title="What to dump" why="Leave schemas and tables empty to dump the whole database. A pattern that matches nothing stops the run, so typos are caught.">
        <div v-if="checks.source?.raw?.schemas?.length">
          <span class="text-sm font-medium text-foreground">Schemas</span>
          <p class="text-xs text-muted-foreground">
            {{ draft.options.schemas?.length ? `${draft.options.schemas.length} selected` : "None selected: every schema is dumped" }}
          </p>
          <div class="mt-2 max-h-64 overflow-auto rounded-lg border border-border">
            <label
              v-for="s in checks.source.raw.schemas"
              :key="s.name"
              class="flex cursor-pointer items-center gap-3 border-b border-border px-3 py-2 text-sm last:border-b-0 hover:bg-muted/50"
            >
              <input type="checkbox" class="rounded border-border" :checked="draft.options.schemas?.includes(s.name)" @change="toggleSchema(draft.options, s.name)" />
              <span class="min-w-0 flex-1 truncate font-mono text-xs">{{ s.name }}</span>
              <span class="text-xs tabular-nums text-muted-foreground">{{ s.tables }} tables</span>
              <span class="w-16 text-right text-xs tabular-nums text-muted-foreground">{{ formatBytes(s.bytes) }}</span>
            </label>
          </div>
        </div>
        <TransferTokens v-else v-model="draft.options.schemas" label="Schemas" help="Test the source connection to pick from a list." placeholder="All schemas" />
        <TransferTokens v-model="draft.include_tables" label="Only these tables" help="pg_dump patterns such as public.orders or sales.*" placeholder="All tables in the chosen schemas" />
        <div>
          <span class="auth-label !mb-1.5">Contents</span>
          <TransferSegmented
            :model-value="contentsOf(draft.options)"
            label="Contents"
            :options="[
              { value: 'all', label: 'Definitions and rows' },
              { value: 'schema', label: 'Definitions only', help: 'Tables, views, functions and indexes without any rows. Useful to prepare an empty copy.' },
              { value: 'data', label: 'Rows only', help: 'Only rows. The destination must already have the tables.' },
            ]"
            @update:model-value="setContents(draft.options, $event)"
          />
        </div>
        <details class="group rounded-lg border border-border" :open="exclusionsCount(draft.options) > 0">
          <summary class="flex cursor-pointer list-none items-center gap-2 px-3 py-2 text-sm font-medium text-foreground">
            <ChevronRight class="h-4 w-4 transition-transform group-open:rotate-90" :stroke-width="1.75" />
            Exclusions<span v-if="exclusionsCount(draft.options)" class="text-xs font-normal text-muted-foreground">({{ exclusionsCount(draft.options) }})</span>
          </summary>
          <div class="space-y-4 border-t border-border p-3">
            <TransferTokens v-model="draft.options.exclude_table_data" label="Skip rows of" help="The table is created, but its rows are left out. Good for audit and log tables." placeholder="public.audit_log" />
            <TransferTokens v-model="draft.options.exclude_tables" label="Leave out tables" placeholder="public.tmp_*" />
            <TransferTokens v-model="draft.options.exclude_schemas" label="Leave out schemas" placeholder="scratch" />
          </div>
        </details>
      </TransferSection>

      <TransferSection v-if="draft.mode !== 'dump_restore'" id="pd-file" title="File" why="Custom archives restore selectively and in parallel with pg_restore. Plain SQL is readable and runs with psql, but restores more slowly.">
        <TransferSegmented
          :model-value="draft.options.format || 'custom'"
          label="Format"
          :options="[
            { value: 'custom', label: 'Custom archive', help: 'Compressed. Restore with pg_restore.' },
            { value: 'plain', label: 'Plain SQL', help: 'A .sql script. Restore with psql.' },
            { value: 'tar', label: 'Tar', help: 'Uncompressed tar archive. Restore with pg_restore.' },
          ]"
          @update:model-value="draft.options.format = $event"
        />
        <label v-if="draft.options.format !== 'tar'" class="block max-w-xs">
          <span class="auth-label !mb-1.5">Compression level</span>
          <input v-model.number="draft.options.compress" type="number" min="0" max="9" class="auth-input tabular-nums" placeholder="Default" />
          <span class="mt-1 block text-xs text-muted-foreground">0 is none, 9 is smallest and slowest.</span>
        </label>
      </TransferSection>

      <TransferSection v-else id="pd-restore" title="Restore behaviour" why="How pg_restore treats objects that already exist in the target database.">
        <TransferSwitch
          v-model="draft.options.restore_clean"
          label="Drop existing objects first"
          help="Each object in the archive is dropped from the target before it is recreated. Objects not in the archive are left alone."
          danger
        />
        <TransferSwitch
          v-model="draft.options.restore_if_exists"
          label="Ignore objects that are not there yet"
          help="Uses DROP … IF EXISTS so a first restore into an empty database is quiet."
          :disabled="!draft.options.restore_clean"
        />
        <TransferSwitch v-model="draft.options.restore_exit_on_error" label="Stop at the first error" help="Off: objects that fail are skipped, the rest is restored, and the run reports how many failed." />
        <TransferSwitch
          v-model="draft.options.restore_single_transaction"
          label="All or nothing"
          help="Restore in a single transaction. Cannot be combined with parallel jobs."
          :disabled="(draft.options.restore_jobs || 1) > 1"
        />
        <TransferSwitch v-model="draft.options.restore_no_owner" label="Do not set owners" help="Objects are owned by the restoring user. Use this when the source roles do not exist on the target." />
        <label class="block max-w-xs">
          <span class="auth-label !mb-1.5">Parallel jobs</span>
          <input v-model.number="draft.options.restore_jobs" type="number" min="1" max="32" class="auth-input tabular-nums" placeholder="1" :disabled="draft.options.restore_single_transaction" />
          <span class="mt-1 block text-xs text-muted-foreground">Tables restored at the same time. Each job uses one connection.</span>
        </label>
      </TransferSection>

      <TransferSection id="pd-sftp" title="Store on SFTP servers" why="After the dump, the file is uploaded to each server under a temporary name, size-checked, then renamed, with a .sha256 file beside it.">
        <p v-if="!draft.destinations.length" class="text-sm text-muted-foreground">No servers. The file stays on the dump volume for download.</p>
        <div class="space-y-2">
          <PgDumpDestination v-for="(d, i) in draft.destinations" :key="i" v-model="draft.destinations[i]" @remove="draft.destinations.splice(i, 1)" />
        </div>
        <AppButton size="sm" :disabled="draft.destinations.length >= 5" :title="draft.destinations.length >= 5 ? 'Up to 5 servers' : ''" @click="draft.destinations.push(emptyDestination())">
          <Plus class="h-3.5 w-3.5" :stroke-width="2" />Add SFTP server
        </AppButton>
        <TransferSwitch
          v-if="draft.destinations.length"
          v-model="draft.options.keep_local"
          label="Keep a copy on the dump volume"
          help="Off: the local file is deleted once every server has a verified copy. If an upload fails, the file is always kept."
        />
      </TransferSection>

      <TransferSection id="pd-advanced" title="Advanced" why="Rarely changed.">
        <details class="group rounded-lg border border-border" :open="advancedCount(draft.options) > 0">
          <summary class="flex cursor-pointer list-none items-center gap-2 px-3 py-2 text-sm font-medium text-foreground">
            <ChevronRight class="h-4 w-4 transition-transform group-open:rotate-90" :stroke-width="1.75" />
            Ownership, privileges and consistency<span v-if="advancedCount(draft.options)" class="text-xs font-normal text-muted-foreground">({{ advancedCount(draft.options) }} on)</span>
          </summary>
          <div class="space-y-4 border-t border-border p-3">
            <TransferSwitch v-model="draft.options.no_owner" label="Leave out ownership" help="The dump does not set object owners. Restoring then works without the source roles." />
            <TransferSwitch v-model="draft.options.no_privileges" label="Leave out privileges" help="No GRANT or REVOKE statements." />
            <TransferSwitch v-model="draft.options.no_comments" label="Leave out comments" />
            <TransferSwitch v-model="draft.options.no_publications" label="Leave out publications" help="Recommended when the source is used for logical replication." />
            <TransferSwitch v-model="draft.options.no_subscriptions" label="Leave out subscriptions" />
            <TransferSwitch
              v-model="draft.options.serializable_deferrable"
              label="Wait for a fully consistent snapshot"
              help="For dumps of busy primaries. The start can wait until no conflicting transactions are running."
            />
            <label class="block max-w-xs">
              <span class="auth-label !mb-1.5">Give up waiting for a table lock after</span>
              <div class="flex items-center gap-2">
                <input
                  :value="lockSeconds(draft.options)"
                  type="number"
                  min="1"
                  class="auth-input tabular-nums"
                  placeholder="Wait forever"
                  @input="setLockSeconds(draft.options, ($event.target as HTMLInputElement).value)"
                />
                <span class="text-sm text-muted-foreground">seconds</span>
              </div>
            </label>
          </div>
        </details>
      </TransferSection>
    </template>

    <template #run-extra="{ job, draft, capability, current }">
      <PgDumpRunExtra
        :job="job"
        :profile-id="current?.id"
        :saved-destinations="current?.destinations || []"
        :retention-hours="Number(capability?.retention_hours) || 0"
        :pinned-names="draft.destinations.filter((d: any) => d.host_key_fingerprint).map((d: any) => d.name || d.host)"
        @pin="(name: string, fp: string) => pin(draft, name, fp)"
      />
    </template>

    <template #last-run-actions="{ job }">
      <a v-if="job.download_path" :href="pgDumpFileUrl(job.id)" class="btn-secondary !h-8 !min-h-8 !w-auto !px-3 !text-xs" download>Download file</a>
    </template>
  </TransferWorkspace>
</template>
