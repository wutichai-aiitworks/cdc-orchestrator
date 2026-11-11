<script setup lang="ts">
// File sync (rsync over SSH) on the shared Copy & Sync workspace. Extras over the other tools:
// SSH hosts instead of databases, a direct-route check, Preview changes, and the relay fallback.
import { ChevronRight, FolderSync, Route } from "@lucide/vue"
import {
  defaultRsyncOptions,
  emptyEndpoint,
  type RsyncEndpoint as Endpoint,
  type RsyncJob,
  type RsyncProfile,
  type RsyncRouteResult,
  useRsync,
} from "~/composables/useRsync"
import { formatBytes } from "~/utils/bytes"
import {
  destructiveEffects,
  liveFacts,
  output,
  routeSteps,
  rsyncChangeLabels,
  runProblems,
  runResult,
  runVerb,
  startedToast,
  summarize,
  transportOf,
} from "~/utils/transferTools/rsync"
import type { TransferTool } from "~/utils/transferTools/types"

definePageMeta({ layout: "default" })

const api = useRsync()
const toast = useToast()
const num = (v: unknown) => (v === null || v === undefined || v === "" || Number.isNaN(Number(v)) ? null : Number(v))

// rsync daemon (rsync://): host, port, module, path inside it, optional daemon user + password.
const cleanDaemon = (e: Endpoint) => ({
  ...e,
  protocol: "daemon" as const,
  module: (e.module || "").trim().replace(/^\/+|\/+$/g, ""),
  name: e.name.trim() || e.host.trim(),
  host: e.host.trim(),
  port: e.port.trim() || "873",
  username: e.username.trim(),
  path: e.path.trim(),
  auth: "password" as const,
  password: e.username.trim() ? e.password : "",
  private_key: "",
  passphrase: "",
  host_key_fingerprint: "",
  rsync_path: "",
  reach_host: "",
  reach_port: "",
})

const cleanEnd = (e: Endpoint) => e.protocol === "daemon" ? cleanDaemon(e) : ({
  ...e,
  protocol: "ssh" as const,
  module: "",
  name: e.name.trim() || e.host.trim(),
  host: e.host.trim(),
  port: e.port.trim() || "22",
  username: e.username.trim(),
  path: e.path.trim(),
  host_key_fingerprint: e.host_key_fingerprint.trim(),
  rsync_path: e.rsync_path.trim(),
  reach_host: e.reach_host.trim(),
  reach_port: e.reach_port.trim(),
  password: e.auth === "password" ? e.password : "",
  private_key: e.auth === "key" ? e.private_key : "",
  passphrase: e.auth === "key" ? e.passphrase : "",
})

const toPayload = (d: any) => ({
  name: d.name.trim(),
  source: { ...cleanEnd(d.source), reach_host: "", reach_port: "" },
  target: cleanEnd(d.target),
  options: {
    ...d.options,
    max_delete: d.options.delete ? num(d.options.max_delete) : null,
    bwlimit_kbps: num(d.options.bwlimit_kbps),
    io_timeout_seconds: num(d.options.io_timeout_seconds),
    excludes: [...(d.options.excludes || [])],
  },
})

const end = (role: string, e: Endpoint, check: any) => ({
  engine: "ssh",
  role: check?.ok && check.raw?.os ? `${role}, ${check.raw.os === "Darwin" ? "macOS" : check.raw.os}` : role,
  title: e.name || e.host,
  detail: e.path,
  mono: true,
})

const tool: TransferTool<RsyncProfile, any, RsyncJob> = {
  key: "rsync",
  title: "File sync",
  icon: FolderSync,
  intro: "Copy a folder from one Linux or macOS host to another with rsync. Only changed files move.",
  emptyText: "A transfer remembers the two hosts, the folders and what to skip. Preview it, then run it whenever the copy should catch up.",
  namePlaceholder: "Web assets to backup Mac",
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
  preview: { label: "Preview changes", startedToast: "Preview started", run: api.previewProfile },
  newDraft: () => ({ name: "", source: emptyEndpoint(), target: emptyEndpoint(), options: defaultRsyncOptions() }),
  toDraft: (p) => ({
    name: p.name,
    source: { ...emptyEndpoint(), ...p.source },
    target: { ...emptyEndpoint(), ...p.target },
    options: { ...defaultRsyncOptions(), ...p.options },
  }),
  toPayload,
  changeLabels: rsyncChangeLabels,
  verb: runVerb,
  startedToast,
  modeLabel: (p) => {
    const t = transportOf(p)
    const route = t === "direct" ? "Direct" : t === "relay" ? "Through relay" : "Direct or relay"
    return `${route}${p.options.delete ? ", mirror with deletes" : ""}${p.options.dry_run ? ", preview only" : ""}`
  },
  runResult,
  steps: (p, job, _log, now) => routeSteps(p, job, now),
  summary: summarize,
  effects: destructiveEffects,
  problems: (job) => runProblems(job),
  jobsOf: (jobs, id) => jobs.filter((j) => j.profile_id === id),
  routeSource: (p, checks) => end("Source", p.source, checks.source),
  routeDestinations: (p, checks) => [
    { ...end("Target", p.target, checks.target), overwrite: p.options.delete && !p.options.dry_run ? "Deletes files not on the source" : undefined },
  ],
  railRoute: (p) => `${p.source.host}:${p.source.path} → ${p.target.host}:${p.target.path}`,
  railEngines: () => ["ssh", "ssh"],
  sections: () => [
    { id: "rs-source", label: "Source host" },
    { id: "rs-target", label: "Target host" },
    { id: "rs-route", label: "Route" },
    { id: "rs-what", label: "What to copy" },
    { id: "rs-advanced", label: "Advanced" },
  ],
  capabilityLine: (c) => {
    const v = String(c.rsync || "").match(/version\s+(\S+)/)?.[1]
    const free = Number(c.free_mb)
    return [v && `rsync ${v}`, Number.isFinite(free) && free >= 0 && `${formatBytes(free * 1024 * 1024)} free for relay copies`].filter(Boolean).join(" · ")
  },
  liveFacts: (j) => liveFacts(j),
  output: (j) => output(j),
}

// ── tests (operate on the slot's draft) ──────────────────────────────────
const tester = (side: "source" | "target", profileId: () => string | undefined) => async (e: Endpoint) => {
  const r = await api.testConnection({ endpoint: cleanEnd(e), side, profile_id: profileId() })
  return r
}
/** Map the endpoint test result into the workspace's checks (route diagram shows the OS). */
const asCheck = (r: any) => (r ? { ok: r.ok, at: new Date().toISOString(), error: r.error, hint: r.hint, raw: r } : null)

const route = ref<RsyncRouteResult | null>(null)
const routeFor = ref("")
const routing = ref(false)
const routeKey = (d: any) => JSON.stringify([cleanEnd(d.source), cleanEnd(d.target)])
const checkRoute = async (d: any, profileId?: string) => {
  routing.value = true
  try {
    route.value = await api.testRoute({ source: cleanEnd(d.source), target: cleanEnd(d.target), profile_id: profileId })
    routeFor.value = routeKey(d)
  } catch (err: any) {
    route.value = { direct: false, transport: "relay", error: err?.data?.error || err?.message || "Check failed" }
    routeFor.value = routeKey(d)
  } finally {
    routing.value = false
  }
}

const TRANSPORTS = [
  { value: "auto", label: "Automatic", help: "Direct when the source can reach the target, otherwise through the relay. Recommended." },
  { value: "direct", label: "Direct only", help: "Fail rather than relay. Needs key login on the target." },
  { value: "relay", label: "Always relay", help: "Pull into the sync service, then push. For hosts that cannot see each other." },
]
const advancedCount = (o: any) =>
  ["checksum", "compress", "no_owner", "hard_links", "acls", "xattrs", "numeric_ids", "one_file_system", "discard_staging"].filter((k) => o[k]).length +
  (o.bwlimit_kbps ? 1 : 0) +
  (o.io_timeout_seconds ? 1 : 0)
const pin = (draft: any, role: "source" | "target", fingerprint: string) => {
  draft[role].host_key_fingerprint = fingerprint
  toast.success(`${role === "source" ? "Source" : "Target"} key pinned. Save to keep it.`)
}
</script>

<template>
  <TransferWorkspace :tool="tool">
    <template #config="{ draft, checks, selectedId, isNew }">
      <TransferSection id="rs-source" title="Source host" why="The host to copy from. The login only needs to read the files.">
        <RsyncEndpoint
          :key="`src-${selectedId}`"
          v-model="draft.source"
          role="source"
          :check="tester('source', () => (isNew ? undefined : selectedId))"
          :start-editing="isNew || !draft.source.host"
          @update:result="checks.source = asCheck($event)"
        />
      </TransferSection>

      <TransferSection id="rs-target" title="Target host" why="The host to copy to. The login must be able to write the folder.">
        <RsyncEndpoint
          :key="`tgt-${selectedId}`"
          v-model="draft.target"
          role="target"
          :check="tester('target', () => (isNew ? undefined : selectedId))"
          :start-editing="isNew || !draft.target.host"
          @update:result="checks.target = asCheck($event)"
        />
      </TransferSection>

      <TransferSection
        id="rs-route"
        title="Route"
        why="Direct copies go from the source straight to the target: the sync service lends the target key to the source for the run, without storing it there. If the hosts cannot see each other, the relay pulls a copy here and pushes it on."
      >
        <TransferSegmented v-model="draft.options.transport" label="Route" :options="TRANSPORTS" />
        <div class="flex flex-wrap items-center gap-3">
          <AppButton size="sm" :disabled="routing || !draft.source.host || !draft.target.host" @click="checkRoute(draft, isNew ? undefined : selectedId)">
            <Route class="h-3.5 w-3.5" :stroke-width="1.75" />{{ routing ? "Checking…" : "Check direct route" }}
          </AppButton>
          <template v-if="route && routeFor === routeKey(draft)">
            <span v-if="route.direct" class="text-sm text-emerald-600 dark:text-emerald-400">The source reaches the target: runs go direct.</span>
            <span v-else class="text-sm text-amber-600 dark:text-amber-400">
              {{ draft.options.transport === "direct" ? "Direct runs will fail: " : "Runs will use the relay: " }}{{ route.error }}
            </span>
          </template>
        </div>
        <p v-if="route && routeFor === routeKey(draft) && !route.direct && route.hint" class="text-xs text-muted-foreground">{{ route.hint }}</p>
        <p v-if="draft.target.auth === 'password' && draft.options.transport !== 'relay'" class="alert-warning text-xs">
          The target signs in with a password, so runs use the relay. Use a key on the target to allow direct copies.
        </p>
      </TransferSection>

      <TransferSection id="rs-what" title="What to copy" why="Only files that differ are sent. Preview changes lists them without writing anything.">
        <TransferTokens v-model="draft.options.excludes" label="Skip" help="rsync patterns such as *.log, .git/ or cache/**" placeholder="Nothing skipped" />
        <TransferSwitch
          v-model="draft.options.delete"
          label="Delete files the source no longer has"
          help="Makes the target an exact mirror. Skipped patterns are left alone on the target."
          danger
        />
        <label v-if="draft.options.delete" class="block max-w-xs">
          <span class="auth-label !mb-1.5">Stop after this many deletions</span>
          <input v-model.number="draft.options.max_delete" type="number" min="0" class="auth-input tabular-nums" placeholder="No limit" />
          <span class="mt-1 block text-xs text-muted-foreground">Guards against an empty or wrong source path wiping the target.</span>
        </label>
        <TransferSwitch v-model="draft.options.dry_run" label="Preview only" help="Every run lists what would change and writes nothing. Turn off when the list looks right." />
      </TransferSection>

      <TransferSection id="rs-advanced" title="Advanced" why="Rarely changed.">
        <details class="group rounded-lg border border-border" :open="advancedCount(draft.options) > 0">
          <summary class="flex cursor-pointer list-none items-center gap-2 px-3 py-2 text-sm font-medium text-foreground">
            <ChevronRight class="h-4 w-4 transition-transform group-open:rotate-90" :stroke-width="1.75" />
            Comparison, attributes and limits<span v-if="advancedCount(draft.options)" class="text-xs font-normal text-muted-foreground">({{ advancedCount(draft.options) }} on)</span>
          </summary>
          <div class="space-y-4 border-t border-border p-3">
            <TransferSwitch v-model="draft.options.checksum" label="Compare by checksum" help="Reads every file on both sides instead of trusting size and time. Slower, catches silent changes." />
            <TransferSwitch v-model="draft.options.compress" label="Compress in transit" help="Helps on slow links; costs CPU on fast ones." />
            <TransferSwitch v-model="draft.options.no_owner" label="Do not keep owners" help="Files belong to the target login. Use when user names differ between the hosts." />
            <TransferSwitch v-model="draft.options.numeric_ids" label="Keep numeric user IDs" help="Do not map owners by name." />
            <TransferSwitch v-model="draft.options.hard_links" label="Keep hard links" />
            <TransferSwitch v-model="draft.options.acls" label="Keep ACLs" help="Needs GNU rsync on both hosts (not the macOS built-in)." />
            <TransferSwitch v-model="draft.options.xattrs" label="Keep extended attributes" help="Needs GNU rsync on both hosts (not the macOS built-in)." />
            <TransferSwitch v-model="draft.options.one_file_system" label="Stay on one file system" help="Do not cross into mounted disks under the source folder." />
            <TransferSwitch
              v-model="draft.options.discard_staging"
              label="Remove the relay copy after each run"
              help="Frees space on the sync service, but the next relay run copies everything again."
            />
            <div class="grid gap-3 sm:grid-cols-2">
              <label class="block">
                <span class="auth-label !mb-1.5">Bandwidth limit (KB/s)</span>
                <input v-model.number="draft.options.bwlimit_kbps" type="number" min="1" class="auth-input tabular-nums" placeholder="Unlimited" />
              </label>
              <label class="block">
                <span class="auth-label !mb-1.5">Give up after no data for (seconds)</span>
                <input v-model.number="draft.options.io_timeout_seconds" type="number" min="10" class="auth-input tabular-nums" placeholder="Never" />
              </label>
            </div>
          </div>
        </details>
      </TransferSection>
    </template>

    <template #run-extra="{ job, draft }">
      <RsyncRunExtra
        :job="job"
        :pinned="{ source: !!draft.source.host_key_fingerprint, target: !!draft.target.host_key_fingerprint }"
        @pin="(role: 'source' | 'target', fp: string) => pin(draft, role, fp)"
      />
    </template>
  </TransferWorkspace>
</template>
