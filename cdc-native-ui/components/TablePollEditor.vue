<script setup lang="ts">
/**
 * Incremental capture: choose how each selected table is polled.
 *
 * - Watermark: JDBC offset on the pipeline's timestamp / incrementing column (default).
 * - Rolling window: every poll re-reads rows whose date column is today or within the
 *   previous N days. For date-only or non-unique timestamps, where a watermark would
 *   skip same-day changes. Older rows need an Initial load.
 * - Full copy: every poll re-reads the whole table. Small tables only.
 *
 * The model only stores non-watermark rules, matching `source.table_poll` in the API.
 */
import type { TablePollRule, TablePollStrategy } from "~/composables/useProfilesM3"

const props = defineProps<{
  tables: string[]
  modelValue: TablePollRule[]
}>()

const emit = defineEmits<{
  "update:modelValue": [value: TablePollRule[]]
}>()

const ruleFor = (table: string): TablePollRule | undefined =>
  props.modelValue.find((r) => r.table.toLowerCase() === table.toLowerCase())

const strategyOf = (table: string): TablePollStrategy => ruleFor(table)?.strategy ?? "watermark"

const update = (table: string, patch: Partial<TablePollRule>) => {
  const rest = props.modelValue.filter((r) => r.table.toLowerCase() !== table.toLowerCase())
  const current: TablePollRule = ruleFor(table) ?? { table, strategy: "watermark" }
  const next: TablePollRule = { ...current, ...patch, table }
  if (next.strategy === "watermark") {
    emit("update:modelValue", rest)
    return
  }
  if (next.strategy === "full_copy") {
    next.window_column = null
    next.window_days = null
  } else if (next.window_days == null) {
    next.window_days = 1
  }
  emit("update:modelValue", [...rest, next])
}

const onStrategy = (table: string, value: string) =>
  update(table, { strategy: value as TablePollStrategy })

const onDays = (table: string, value: string) => {
  const n = Number.parseInt(value, 10)
  update(table, { window_days: Number.isFinite(n) && n >= 0 ? Math.min(n, 31) : 1 })
}

const windowLabel = (rule: TablePollRule | undefined) => {
  const days = rule?.window_days ?? 1
  return days === 0 ? "today only" : `today + previous ${days} day${days === 1 ? "" : "s"}`
}
</script>

<template>
  <div class="space-y-2">
    <AppFormLabel
      label="Poll strategy per table"
      hint="Watermark uses the timestamp/incrementing column above. Rolling window re-reads recent rows on every poll — use it when the date column has no time part or is not unique; run Initial load once for older rows. Full copy re-reads the whole table on every poll (small tables only). Hard deletes are not captured by any of them."
    />
    <p v-if="!tables.length" class="text-xs text-muted-foreground">Select tables first.</p>
    <div
      v-for="table in tables"
      :key="table"
      class="grid grid-cols-1 gap-2 rounded-lg border border-border p-3 sm:grid-cols-[minmax(0,1fr)_12rem] sm:items-center"
    >
      <span class="truncate font-mono text-xs text-foreground" :title="table">{{ table }}</span>
      <AppSelect
        :model-value="strategyOf(table)"
        :aria-label="`Poll strategy for ${table}`"
        @update:model-value="(v: string) => onStrategy(table, v)"
      >
        <option value="watermark">Watermark</option>
        <option value="rolling_window">Rolling window</option>
        <option value="full_copy">Full copy each poll</option>
      </AppSelect>
      <div
        v-if="strategyOf(table) === 'rolling_window'"
        class="grid grid-cols-1 gap-2 sm:col-span-2 sm:grid-cols-[minmax(0,1fr)_8rem]"
      >
        <label>
          <span class="auth-label">Date column</span>
          <AppInput
            :model-value="ruleFor(table)?.window_column ?? ''"
            placeholder="UPDATE_DATE"
            @update:model-value="(v: string | number) => update(table, { window_column: String(v).trim() || null })"
          />
        </label>
        <label>
          <span class="auth-label">Previous days</span>
          <AppInput
            :model-value="String(ruleFor(table)?.window_days ?? 1)"
            inputmode="numeric"
            @update:model-value="(v: string | number) => onDays(table, String(v))"
          />
        </label>
        <p class="text-xs text-muted-foreground sm:col-span-2">
          Every poll re-reads rows where the date column is {{ windowLabel(ruleFor(table)) }}.
        </p>
      </div>
      <p v-else-if="strategyOf(table) === 'full_copy'" class="text-xs text-muted-foreground sm:col-span-2">
        Every poll re-reads the whole table.
      </p>
    </div>
  </div>
</template>
