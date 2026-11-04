# Copy & Sync tools — UX/UI redesign

Scope: the five one-shot transfer tools in the native UI — Postgres copy (pgcopydb), Postgres dump
(pg_dump / pg_restore / SFTP), Load to Postgres (pgloader), Mongo copy (mongodump / mongorestore),
Redis sync (RedisShake). Mockup: [`migration-tools-mockup.html`](migration-tools-mockup.html)
(`?view=overview|config|runs`).

Status: implemented 2026-11-03 for all five tools. One shell, `components/transfer/TransferWorkspace.vue`,
driven by a per-page descriptor (`utils/transferTools/types.ts`); tool rules are pure and unit-tested
(`utils/pgDumpView.ts`, `utils/transferTools/{pgcopy,pgloader,mongocopy,redisshake}.ts`, shared
`utils/transferView.ts`). Differences from the proposal below:

- Every tool has a real **Test connection** from its sidecar and keeps **run history**
  (ADR 0006). Steps come from the run's log (pgcopydb `STEP n`, pgloader phases, mongorestore
  lines, RedisShake `syncing aof`).
- Redis sync is continuous: "Start sync" / "Stop sync"; Overview shows keys written and lag.
- No MongoDB/Redis/CSV logos exist in the app: those ends use a neutral lettered tile.

## Why change

What the five pages do today, and what it costs the operator:

| Today | Effect |
|---|---|
| One long form per page; options are a flat wall of checkboxes (pgcopy: 25, "drop if exists" beside "skip analyze") | Dangerous settings look as harmless as cosmetic ones |
| Save, Run, Delete are equal full-width orange bars | No obvious next step; Delete has the same weight as Run |
| "Run" silently saves the form first | A half-edited profile can run against production |
| Profile list shows name and hosts only | Can't see which transfer failed or when it last ran |
| One jobs table for all profiles, ID + status only | Can't tell whose job it is, how long it took, what it produced |
| Logs are a `<pre>` under the table, not tied to a row | Errors are hard to connect to the run that caused them |
| "Validate" only checks that fields are filled | Looks like a connection test; isn't |
| Each page names actions differently ("Run clone", "Run copy", "Run dump") | No shared vocabulary across tools |
| Pages ignore the app's UI kit (AppButton, AppStatusBadge, AppConfirmDialog, AppDbIcon) | Looks unlike the rest of the product |

## Design principles

1. **A transfer is a route.** Every saved profile is shown as *source → steps → destination(s)*,
   with real database logos. That diagram (the Route) is the one distinctive element; everything
   else stays quiet.
2. **One primary action per view.** The header's Run (or Stop while running) is the only orange
   button. Save appears only in a sticky bar when there are unsaved changes.
3. **Never run unsaved changes.** Run is disabled while the form is dirty ("Save your changes
   first"), instead of saving implicitly.
4. **Say what happens to the destination.** Options are described by their outcome and grouped;
   anything that deletes or replaces data on the destination is marked, and a run that does so gets
   a confirmation that lists the effects.
5. **Runs belong to a transfer.** Each run has phases, numbers, its own log, and a plain-language
   problem + fix when it fails.
6. **Same words everywhere.** The verb on the button, in the toast, in the run row and in the
   audit log is the same.

## Visual system

Unchanged product tokens (`assets/css/main.css`): Inter; `--primary` orange as the signal colour;
neutral surfaces; the existing status palette (emerald / red / amber / blue, as `badge-*`).

| Token | Use |
|---|---|
| `--foreground` | Text, the Route's static line |
| `--background` / `--card` | The single working surface (detail pane) |
| `--muted` | Rail background, read-only values, code |
| `--border` | Section dividers — sections are separated by rules and space, **not nested cards** |
| `--primary` | One primary button per view; the Route line animates in this colour only while a run is live (static under `prefers-reduced-motion`) |
| amber | "Replaces data on the target" marks, warnings, unsaved changes |

Type scale (Inter only): 20/28 semibold page title (top bar) · 16/24 semibold transfer name ·
13/20 semibold section heads, sentence case · 14/20 body · 12/16 help and secondary text.
Tabular numerals for sizes, counts and durations. Monospace only for strings people copy: paths,
table patterns, host-key fingerprints, log lines.

Radii: 8px for panels (Route, run detail, log), 6px for inputs and buttons, 12px pills for status.

## Layout

```
┌ top bar: Postgres dump ───────────────────────────────────────────────────────────┐
├ rail 288px ──────────┬ detail ─────────────────────────────────────────────────────┤
│ purpose line   [New] │ Transfer name                              [Run dump] [⋯]   │
│ [Find a transfer]    │ mode   last run                                             │
│ ┌──────────────────┐ │ ┌ Route ──────────────────────────────────────────────────┐ │
│ │[pg][pg] Orders…  │ │ │ [pg] Source host     ①Dump ─ ②Upload ─   SFTP host [SF] │ │
│ │ prod → replica   │ │ │      db, port        (live line while running)          │ │
│ │ ● Running 42%    │ │ └──────────────────────────────────────────────────────────┘ │
│ └──────────────────┘ │ Overview   Configuration   Runs 31                          │
│  Shop nightly        │ ─────────────────────────────────────────────────────────── │
│  ● Succeeded 2 h ago │  tab content (max 1120px, left-aligned)                     │
└──────────────────────┴─────────────────────────────────────────────────────────────┘
```

- **Rail**: one row per transfer — source/destination logos, name, short route, and the last
  result with relative time (`● Running, restore 42%`, `● Upload failed yesterday`, `● Never run`).
  Search filters by name or host. Below 1024px the rail becomes a select above the detail.
- **Overview tab**: while running — facts strip (started, elapsed, tables, size), progress bar, the
  latest log line; always — "What this transfer does" in plain sentences, last success.
- **Configuration tab**: sections in two columns (title + why on the left, fields on the right),
  section index on the right that tracks scroll, sticky save bar listing what changed.
- **Runs tab**: table (result, started, duration, output, stored on). Clicking a row expands it:
  phase strip, problem + fix with a one-click remedy (e.g. *Retry upload*), log tail with Copy /
  Open full log. A running row auto-expands.
- **New**: a small dialog — name, and for tools with modes the mode — then lands on Configuration.
- **Empty states**: no transfers → one sentence on what the tool does + "New transfer". Sidecar
  not configured → the env vars to set (`ORCH_<TOOL>_URL`) instead of a broken form.

## Shared components (new, in `components/transfer/`)

| Component | Responsibility |
|---|---|
| `TransferWorkspace` | Rail + header + Route + tabs; polling (3 s while a run is active, 30 s otherwise); URL state `?t=<id>&tab=runs&run=<id>` so links are shareable |
| `TransferRail` | List, search, last-result line, keyboard navigation (↑/↓, Enter) |
| `TransferRoute` | Endpoints with `AppDbIcon`, step markers (pending / now / done / failed), live line, "Replaces existing objects" mark |
| `ConnectionCard` / `ConnectionFields` | Read view with last test result; edit view per engine (Postgres, MySQL/MariaDB, SQL Server, SQLite file, CSV, MongoDB, Redis) with engine default ports, show/hide secret, **Test connection** |
| `OptionSection` | Renders a section from a descriptor: switch rows with outcome text, number fields with units, segmented choices; `danger: true` adds the amber "replaces data on the target" note |
| `RunTable` / `RunDetail` | Rows, phase strip, problem + fix, actions (Download, Retry upload, Stop) |
| `LogViewer` | Tail with follow, wrap toggle, copy, full-log dialog; highlights `error:` / `warning:` lines |
| `RunConfirmDialog` | Built on `AppConfirmDialog`; lists destructive effects computed from the profile |
| `SaveBar` | Dirty tracking against the loaded profile, field-level change list, Discard / Save changes |

Each tool page becomes a descriptor + `<TransferWorkspace :tool="pgdumpTool" />`
(`utils/transferTools/*.ts`): connections, route steps, option sections, verbs, destructive rules,
and how a job maps to phases. Both UI trees get the same files (parity script).

## Per-tool mapping

| | Postgres copy | Postgres dump | Load to Postgres | Mongo copy | Redis sync |
|---|---|---|---|---|---|
| Run verb (button, toast, run row) | Copy database | Run dump | Load into Postgres | Copy MongoDB | Start sync / Stop sync |
| Route steps | Schema → Data → Indexes → (Follow changes) | Dump → (Upload) → (Restore) | Read source → Create tables → Load → Indexes | Dump → Restore | Full sync → Live sync |
| Destination | Postgres | File, SFTP servers, or Postgres | Postgres | MongoDB | Redis |
| Sections | Source · Target · What to copy · Speed · Target behaviour · Skip steps · Live follow (advanced) · Recovery (advanced) | Source · (Target) · What to dump · File · Store on SFTP servers · Restore behaviour · Advanced | Source (engine picker with logos) · Target · What to load · Target behaviour · CSV format (only for CSV) · Speed | Source · Target · What to copy · Target behaviour | Source · Target · What to sync · When a key already exists |
| Marked as replacing data | drop if exists | clean, restore clean | truncate, drop if exists | drop collections | duplicate keys: rewrite |
| Overview while running | tables copied / total, indexes built | per-phase progress, archive size, uploads | rows loaded per table | collections restored | lag, keys synced; "Live" is the normal running state, not "in progress" |

Option copy is written as outcomes, e.g. pgcopy `skip_analyze` → "Skip ANALYZE after loading —
faster, but the planner has no statistics until autovacuum runs"; Redis `rdb_restore_command_behavior`
→ a three-way choice: "Overwrite the existing key" (marked), "Keep the existing key",
"Stop the sync".

## Interaction rules

- Run disabled while dirty; Stop replaces Run while a run is active; ⋯ menu holds Duplicate,
  Export JSON, Delete (with `AppConfirmDialog`, danger).
- A run whose profile has any destructive option opens `RunConfirmDialog`: "This will, on
  replica-01 / orders: drop and recreate 88 tables in sales, ops." Primary button repeats the verb.
- Secrets render as "Saved" with Replace; never echoed, never in the DOM after save.
- Errors: what happened + what to do, never "Something went wrong". Reuse the sidecars' `hint`,
  `first_error`, per-upload `hint`.
- Toasts confirm with the same verb: "Dump started", "Changes saved", "Upload retried".

## Accessibility and quality floor

Keyboard-reachable rail and tabs (`role="tablist"`), visible focus ring (`ring-1 ring-ring`), status
never by colour alone (icon + text), live region for run status changes, reduced motion respected,
dark theme via existing tokens (no hard-coded colours), usable at 1024px and down to a single
column on narrow screens.

## Backend work the design depends on

| Need | Change |
|---|---|
| Real **Test connection** | Sidecar `POST /v1/test-connection` per tool (pg: `SELECT version()`, mongo: `ping`, redis: `PING` + `INFO server`, SFTP: connect + `stat remote_dir`); returns version and latency |
| Runs per transfer | All sidecars record `profile_id` on jobs (pgdump does; pgcopy uses the profile id as job id; others need it) and support `GET /v1/jobs?profile_id=` |
| Progress numbers | Expose per-tool counters on job status (pgdump has tables/bytes; pgcopydb `pgcopydb list progress`; pgloader summary; RedisShake status endpoint) |
| Retry upload (pgdump) | `POST /cdc/pgdump/jobs/{id}/upload` re-sending the profile's destinations to the sidecar |
| Destructive summary | Computed client-side from the descriptor; no backend change |

## Rollout

1. Shared components + Postgres dump (richest tool; validates the pattern).
2. Mongo copy and Load to Postgres (closest to dump).
3. Postgres copy (largest option set), then Redis sync (continuous-run semantics).
4. Backend items in parallel; until Test connection exists the button reads "Check fields".
