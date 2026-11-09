// What a Copy & Sync page hands to <TransferWorkspace>. Pure rules come from
// utils/transferTools/<tool>.ts (unit-tested); the page adds its API calls and icon.
import type { Component } from "vue"
import type { BaseJob, Problem, RouteStep, SummaryRow, Tone } from "../transferView"

export type StartMode<J = any> = {
  key: string
  label: string
  help: string
  startedToast: string
  primary?: boolean
  /** What this kind of run changes (confirm dialog); default: the tool's effects. */
  effects?: string[]
  run: (profileId: string) => Promise<J>
}

/** One end of the route diagram. `engine` picks the logo (postgres, mysql, mongodb, redis, csv, sftp, file…). */
export type RouteEnd = { engine: string; role: string; title: string; detail?: string; mono?: boolean; /** Warning shown under the end, e.g. "Replaces existing objects". */ overwrite?: string }

/** Result of a connection check shown under a connection. `raw` keeps tool data (e.g. schemas). */
export type ConnCheck = {
  ok: boolean
  at: string
  facts?: string[]
  warning?: string
  error?: string
  hint?: string | null
  raw?: any
}

export type TransferApi<P, J> = {
  fetchProfiles: () => Promise<P[]>
  createProfile: (body: any) => Promise<P>
  updateProfile: (id: string, body: any) => Promise<P>
  deleteProfile: (id: string) => Promise<unknown>
  runProfile: (id: string) => Promise<J>
  fetchJobs: () => Promise<J[]>
  fetchJobLogs: (id: string) => Promise<{ log: string }>
  stopJob: (id: string) => Promise<unknown>
  /** Only tools whose runs can be deleted (pgdump). */
  deleteJob?: (id: string) => Promise<unknown>
  fetchCapability?: () => Promise<Record<string, any>>
}

export type TransferTool<P extends { id: string; name: string } = any, D = any, J extends BaseJob = any> = {
  key: string
  /** Short name used in sentences: "Postgres copy". */
  title: string
  icon: Component
  /** One sentence under the page title. */
  intro: string
  emptyText: string
  namePlaceholder: string
  /** What one saved profile is called in the UI. Default { one: "transfer", many: "transfers" }. */
  noun?: { one: string; many: string }
  /** Save-bar button for a profile that is not saved yet. Default "Create <noun>". */
  createLabel?: string
  /** pgcopy/pgloader/mongocopy/redisshake keep only the latest run (job id = profile id). */
  latestOnly?: boolean
  /** Redis sync: running is the normal state, Stop ends it. */
  continuous?: boolean
  stopVerb?: string
  api: TransferApi<P, J>

  newDraft: () => D
  toDraft: (p: P) => D
  /** API body for create/update. Also what changes, the route and the summary are computed from. */
  toPayload: (d: D) => any
  changeLabels: [string, (payload: any) => unknown][]

  verb: (payload: any) => string
  startedToast: (payload: any) => string
  modeLabel: (payload: any) => string
  /** `log` (when loaded) lets a continuous tool say which phase a live run is in. */
  runResult: (job: J, log?: string) => { tone: Tone; text: string }
  steps: (payload: any, job: J | undefined, log: string, now: number) => RouteStep[]
  summary: (payload: any) => SummaryRow[]
  effects: (payload: any) => string[]
  problems: (job: J, log: string) => Problem[]
  jobsOf: (jobs: J[], profileId: string) => J[]

  routeSource: (payload: any, checks: Record<string, ConnCheck | null>) => RouteEnd
  routeDestinations: (payload: any, checks: Record<string, ConnCheck | null>, capability: Record<string, any> | null) => RouteEnd[]
  railRoute: (p: P) => string
  railEngines: (p: P) => [string, string?]
  sections: (payload: any) => { id: string; label: string }[]
  capabilityLine?: (cap: Record<string, any>) => string
  /** Extra numbers for a live run on Overview (e.g. tables dumped). */
  liveFacts?: (job: J, log: string) => { label: string; value: string }[]
  /** Short "what it produced" for the run table and last-run line. */
  output?: (job: J) => string
  /**
   * Ways to start a run (Redis sync: CDC or one-time sync). The `primary` one is the main button;
   * the others sit in a "Start as" menu beside it. Each starts one run; the profile is unchanged.
   */
  startModes?: (payload: any) => StartMode<J>[]
  /** Optional second action beside Run that never changes data (File sync: "Preview changes"). */
  preview?: { label: string; startedToast: string; run: (profileId: string) => Promise<J> }
}
