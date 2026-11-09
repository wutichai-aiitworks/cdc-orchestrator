<script setup lang="ts">
import { transferTester } from "~/composables/useTransferTest"
// Mongo copy (mongodump → mongorestore) on the shared Copy & Sync workspace.
import { Database } from "@lucide/vue"
import { applyContractOptionDefaults } from "~/composables/useMigrationTools"
import { defaultMongoCopyOptions, type MongoCopyConn, type MongoCopyJob, type MongoCopyProfile, useMongoCopy } from "~/composables/useMongoCopy"
import * as rules from "~/utils/transferTools/mongocopy"
import type { TransferTool } from "~/utils/transferTools/types"

definePageMeta({ layout: "default" })

const api = useMongoCopy()
const contract = ref<Record<string, unknown> | null>(null)
onMounted(() => api.fetchContract().then((c) => (contract.value = c)).catch(() => {}))

const emptyConn = (): MongoCopyConn => ({ type: "mongodb", host: "", port: "27017", database: "", username: "", password: "" })
const toPayload = (d: any) => ({
  name: d.name.trim(),
  source: { ...d.source, type: "mongodb" },
  target: { ...d.target, type: "mongodb" },
  include_collections: [...d.include_collections],
  options: { ...d.options, auth_source: d.options.auth_source?.trim() || "" },
})
const test = transferTester("mongocopy")
const end = (role: string, c: MongoCopyConn, overwrite?: string) => ({
  engine: "mongodb",
  role,
  title: c.host,
  detail: `${c.database || "all databases"}, port ${c.port || "27017"}`,
  overwrite,
})

/** ", PostgreSQL 17" once a connection test has run (the route then names the server). */
const ver = (check: any, name: string) => (check?.ok && check.raw?.server_version ? `, ${name} ${String(check.raw.server_version).split(" ")[0]}` : "")

const tool: TransferTool<MongoCopyProfile, any, MongoCopyJob> = {
  key: "mongocopy",
  title: "Mongo copy",
  icon: Database,
  intro: "Copy MongoDB databases or collections from one server to another with mongodump and mongorestore.",
  noun: { one: "copy", many: "copies" },
  emptyText: "A copy dumps a MongoDB database (or chosen collections) and restores it on another server, optionally under a new database name.",
  namePlaceholder: "App data to staging",
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
    include_collections: [],
    options: applyContractOptionDefaults(defaultMongoCopyOptions(), contract.value ?? undefined),
  }),
  toDraft: (p) => ({
    name: p.name,
    source: { ...emptyConn(), ...p.source },
    target: { ...emptyConn(), ...p.target },
    include_collections: [...(p.include_collections || [])],
    options: { ...defaultMongoCopyOptions(), ...p.options },
  }),
  toPayload,
  changeLabels: rules.changeLabels,
  verb: rules.verb,
  startedToast: rules.startedToast,
  modeLabel: (p) => (rules.renames(p) ? `Copy and rename to ${p.target.database}` : "MongoDB to MongoDB"),
  runResult: rules.runResult,
  steps: (p, job, log) => rules.steps(p, job, log),
  summary: rules.summary,
  effects: rules.effects,
  problems: rules.problems,
  // Runs before per-run job ids used the profile id as the job id.
  jobsOf: (jobs, id) => jobs.filter((j) => (j.profile_id || j.id) === id),
  routeSource: (p, checks) => end(`Source${ver(checks.source, "MongoDB")}`, p.source),
  routeDestinations: (p, checks) => [end(`Target${ver(checks.target, "MongoDB")}`, p.target, p.options.drop ? "Replaces copied collections" : undefined)],
  railRoute: (p) => `${p.source.host}/${p.source.database || "*"} → ${p.target.host}/${p.target.database || "*"}`,
  railEngines: () => ["mongodb", "mongodb"],
  sections: () => [
    { id: "mc-source", label: "Source server" },
    { id: "mc-target", label: "Target server" },
    { id: "mc-scope", label: "What to copy" },
    { id: "mc-behaviour", label: "Target behaviour" },
  ],
  capabilityLine: (c) => [c.mongodump_version, c.mongorestore_version].filter(Boolean).map((v) => String(v).split("\n")[0]).join(" · "),
}
const CHECK_NOTE = "Tested from the mongocopy container, the same way a run connects."
</script>

<template>
  <TransferWorkspace :tool="tool">
    <template #config="{ draft, checks, savedLogin, selectedId, isNew }">
      <TransferSection id="mc-source" title="Source server" why="The MongoDB server to copy from. Leave the database empty to copy every database. A user with the backup role is enough.">
        <TransferConnection
          :key="`src-${selectedId}`"
          v-model="draft.source"
          v-model:result="checks.source"
          engine="mongodb"
          default-port="27017"
          host-placeholder="mongo.example.internal"
          :database="{ label: 'Database', placeholder: 'All databases' }"
          username-optional
          username-placeholder="backup"
          :check="test('source', () => ({ auth_source: draft.options.auth_source }), () => (isNew ? undefined : selectedId))"
          :check-note="CHECK_NOTE"
          :watch-key="draft.options.auth_source"
          :saved-login="savedLogin('source', '27017')"
          :start-editing="isNew || !draft.source.host"
        />
      </TransferSection>

      <TransferSection id="mc-target" title="Target server" why="Where the copy is restored. A different database name renames everything into it.">
        <TransferConnection
          :key="`tgt-${selectedId}`"
          v-model="draft.target"
          v-model:result="checks.target"
          engine="mongodb"
          default-port="27017"
          host-placeholder="mongo-staging.example.internal"
          :database="{ label: 'Database', placeholder: 'Same as source' }"
          username-optional
          username-placeholder="restore"
          :check="test('target', () => ({ auth_source: draft.options.auth_source }), () => (isNew ? undefined : selectedId))"
          :check-note="CHECK_NOTE"
          :watch-key="draft.options.auth_source"
          :saved-login="savedLogin('target', '27017')"
          :start-editing="isNew || !draft.target.host"
        />
        <label class="block max-w-xs">
          <span class="auth-label !mb-1.5">Authentication database</span>
          <input v-model="draft.options.auth_source" class="auth-input" placeholder="admin" spellcheck="false" />
          <span class="mt-1 block text-xs text-muted-foreground">Where the users are defined, on both servers. Usually admin.</span>
        </label>
      </TransferSection>

      <TransferSection id="mc-scope" title="What to copy" why="Leave empty to copy every collection. Picking collections turns off renaming into a different database.">
        <TransferTokens v-model="draft.include_collections" label="Only these collections" help="collection, or database.collection" placeholder="All collections" />
        <p v-if="rules.renames(draft)" class="text-xs text-muted-foreground">
          Everything in {{ draft.source.database }} is restored into {{ draft.target.database }}.
        </p>
        <TransferSwitch v-model="draft.options.gzip" label="Compress the archive" help="Smaller and usually faster over the network. Both tools handle it automatically." />
      </TransferSection>

      <TransferSection id="mc-behaviour" title="Target behaviour" why="What happens to collections that already exist on the target.">
        <TransferSwitch
          v-model="draft.options.drop"
          label="Replace copied collections"
          help="Each copied collection is dropped on the target before it is restored. Off: documents are added; ones whose _id already exists are skipped and reported."
          danger
        />
      </TransferSection>
    </template>
  </TransferWorkspace>
</template>
