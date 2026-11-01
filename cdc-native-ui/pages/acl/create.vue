<script setup lang="ts">
import { formatApiError } from "~/utils/apiError"

const router = useRouter()
const toast = useToast()
const { createAcl } = useKafkaAdmin()

const principal = ref("User:")
const host = ref("*")
const resourceType = ref("topic")
const resourceName = ref("")
const patternType = ref("literal")
const operation = ref("all")
const permission = ref("allow")
const submitting = ref(false)
const error = ref("")

const RESOURCE_TYPES = [
  { value: "topic", label: "Topic", hint: "A specific topic", icon: "T" },
  { value: "group", label: "Group", hint: "Consumer group", icon: "G" },
  { value: "cluster", label: "Cluster", hint: "Cluster-wide", icon: "C" },
  { value: "transactional_id", label: "Txn ID", hint: "Transactional producer", icon: "X" },
  { value: "delegation_token", label: "Token", hint: "Delegation token", icon: "D" },
]

const OPERATIONS: Record<string, { value: string; label: string }[]> = {
  topic: [
    { value: "all", label: "ALL" },
    { value: "read", label: "READ" },
    { value: "write", label: "WRITE" },
    { value: "create", label: "CREATE" },
    { value: "delete", label: "DELETE" },
    { value: "alter", label: "ALTER" },
    { value: "describe", label: "DESCRIBE" },
    { value: "describe_configs", label: "DESCRIBE_CONFIGS" },
    { value: "alter_configs", label: "ALTER_CONFIGS" },
  ],
  group: [
    { value: "all", label: "ALL" },
    { value: "read", label: "READ" },
    { value: "describe", label: "DESCRIBE" },
    { value: "delete", label: "DELETE" },
  ],
  cluster: [
    { value: "all", label: "ALL" },
    { value: "create", label: "CREATE" },
    { value: "describe", label: "DESCRIBE" },
    { value: "alter", label: "ALTER" },
    { value: "clusteraction", label: "CLUSTER_ACTION" },
    { value: "describe_configs", label: "DESCRIBE_CONFIGS" },
    { value: "alter_configs", label: "ALTER_CONFIGS" },
    { value: "idempotent_write", label: "IDEMPOTENT_WRITE" },
  ],
  transactional_id: [
    { value: "all", label: "ALL" },
    { value: "describe", label: "DESCRIBE" },
    { value: "write", label: "WRITE" },
  ],
  delegation_token: [
    { value: "all", label: "ALL" },
    { value: "describe", label: "DESCRIBE" },
  ],
}

const availableOps = computed(() => OPERATIONS[resourceType.value] || OPERATIONS.topic)

watch(resourceType, () => {
  const ops = OPERATIONS[resourceType.value] || OPERATIONS.topic
  if (!ops.find(o => o.value === operation.value)) {
    operation.value = "all"
  }
})

const submit = async () => {
  if (!principal.value.trim()) {
    error.value = "Principal is required"
    return
  }
  if (resourceType.value !== "cluster" && !resourceName.value.trim()) {
    error.value = "Resource name is required"
    return
  }
  submitting.value = true
  error.value = ""
  try {
    await createAcl({
      principal: principal.value.trim(),
      host: host.value.trim() || "*",
      resource_type: resourceType.value,
      resource_name: resourceType.value === "cluster" ? "kafka-cluster" : resourceName.value.trim(),
      pattern_type: patternType.value,
      operation: operation.value,
      permission: permission.value,
    })
    toast.success("ACL created")
    await router.push("/acl")
  } catch (e: any) {
    error.value = formatApiError(e, "Failed to create ACL")
    toast.error(error.value)
  } finally {
    submitting.value = false
  }
}
</script>

<template>
  <div class="page-container">
    <AppPageHeader title="Create ACL" description="Define Kafka access control rules for principals, resources, and operations.">
      <AppButton as="link" to="/acl">Cancel</AppButton>
      <AppButton variant="primary" :disabled="submitting" @click="submit">
        {{ submitting ? "Creating…" : "Create ACL" }}
      </AppButton>
    </AppPageHeader>

    <AppStateBlock v-if="error" kind="error" :message="error" class="mb-4" />

    <div class="grid grid-cols-1 gap-5 lg:grid-cols-3">
      <!-- Left column: principal + resource -->
      <div class="flex flex-col gap-5 lg:col-span-2">
        <!-- Principal + Host -->
        <AppCard class="space-y-5">
          <h2 class="section-title">Identity</h2>
          <div class="grid grid-cols-1 gap-4 sm:grid-cols-2">
            <label>
              <span class="auth-label">Principal <span class="text-red-400">*</span></span>
              <AppInput v-model="principal" placeholder="User:alice" />
              <span class="mt-1 block text-xs text-surface-400">Format: <code class="font-mono">User:name</code> or <code class="font-mono">*</code> for all</span>
            </label>
            <label>
              <span class="auth-label">Host</span>
              <AppInput v-model="host" placeholder="*" />
              <span class="mt-1 block text-xs text-surface-400">IP or hostname. Use <code class="font-mono">*</code> for any host.</span>
            </label>
          </div>
        </AppCard>

        <!-- Resource -->
        <AppCard class="space-y-5">
          <h2 class="section-title">Resource</h2>

          <!-- Resource type card selector -->
          <div>
            <span class="auth-label">Resource type <span class="text-red-400">*</span></span>
            <div class="mt-1.5 grid grid-cols-5 gap-2">
              <button
                v-for="rt in RESOURCE_TYPES"
                :key="rt.value"
                type="button"
                :class="[
                  'flex flex-col items-center gap-1 rounded-xl border px-2 py-3 text-xs font-medium transition-all',
                  resourceType === rt.value
                    ? 'border-brand-500 bg-brand-500/10 text-brand-400'
                    : 'border-surface-700 bg-surface-800/50 text-surface-300 hover:border-surface-500 hover:text-surface-100',
                ]"
                @click="resourceType = rt.value"
              >
                <span class="text-base font-bold">{{ rt.icon }}</span>
                <span class="font-semibold">{{ rt.label }}</span>
                <span class="text-center text-[10px] leading-tight opacity-60">{{ rt.hint }}</span>
              </button>
            </div>
          </div>

          <!-- Resource name -->
          <label v-if="resourceType !== 'cluster'">
            <span class="auth-label">Resource name <span class="text-red-400">*</span></span>
            <AppInput v-model="resourceName" :placeholder="`Enter ${resourceType} name`" />
            <span v-if="patternType === 'prefixed'" class="mt-1 block text-xs text-surface-400">
              Prefix match — e.g. <code class="font-mono">demo.</code> matches all topics starting with "demo."
            </span>
          </label>
          <div v-else class="rounded-xl border border-surface-700/50 bg-surface-800/30 px-4 py-3 text-sm text-surface-300">
            Cluster resource uses fixed name <code class="font-mono text-brand-400">kafka-cluster</code>.
          </div>

          <!-- Pattern type -->
          <div>
            <span class="auth-label">Pattern type</span>
            <div class="mt-1.5 flex gap-2">
              <button
                v-for="pt in [
                  { value: 'literal', label: 'LITERAL', desc: 'Exact match' },
                  { value: 'prefixed', label: 'PREFIXED', desc: 'Starts with' },
                  { value: 'match', label: 'MATCH', desc: 'Wildcard' },
                ]"
                :key="pt.value"
                type="button"
                :class="[
                  'flex flex-1 flex-col items-center gap-0.5 rounded-xl border px-3 py-2 text-xs font-medium transition-all',
                  patternType === pt.value
                    ? 'border-brand-500 bg-brand-500/10 text-brand-400'
                    : 'border-surface-700 bg-surface-800/50 text-surface-300 hover:border-surface-500',
                ]"
                @click="patternType = pt.value"
              >
                <span class="font-semibold">{{ pt.label }}</span>
                <span class="text-[10px] opacity-60">{{ pt.desc }}</span>
              </button>
            </div>
          </div>
        </AppCard>

        <!-- Operation + Permission -->
        <AppCard class="space-y-5">
          <h2 class="section-title">Access</h2>

          <!-- Operations grid -->
          <div>
            <span class="auth-label">Operations <span class="text-red-400">*</span></span>
            <div class="mt-1.5 flex flex-wrap gap-2">
              <button
                v-for="op in availableOps"
                :key="op.value"
                type="button"
                :class="[
                  'rounded-lg border px-3 py-1.5 text-xs font-medium transition-all',
                  operation === op.value
                    ? 'border-brand-500 bg-brand-500/10 text-brand-400'
                    : 'border-surface-700 bg-surface-800/40 text-surface-300 hover:border-surface-500',
                ]"
                @click="operation = op.value"
              >
                {{ op.label }}
              </button>
            </div>
          </div>

          <!-- Permission toggle -->
          <div>
            <span class="auth-label">Permission</span>
            <div class="mt-1.5 flex gap-3">
              <button
                v-for="perm in [
                  { value: 'allow', label: 'ALLOW', cls: 'border-emerald-500 bg-emerald-500/10 text-emerald-400' },
                  { value: 'deny', label: 'DENY', cls: 'border-red-500 bg-red-500/10 text-red-400' },
                ]"
                :key="perm.value"
                type="button"
                :class="[
                  'flex flex-1 items-center justify-center rounded-xl border px-4 py-2.5 text-sm font-semibold transition-all',
                  permission === perm.value
                    ? perm.cls
                    : 'border-surface-700 bg-surface-800/50 text-surface-300 hover:border-surface-500',
                ]"
                @click="permission = perm.value"
              >
                {{ perm.label }}
              </button>
            </div>
          </div>
        </AppCard>
      </div>

      <!-- Right column: preview + help -->
      <div class="flex flex-col gap-5">
        <!-- ACL preview -->
        <AppCard class="space-y-3">
          <h2 class="section-title">Preview</h2>
          <div class="rounded-xl border border-surface-700/50 bg-surface-950/60 p-4 font-mono text-xs leading-relaxed">
            <div class="mb-2 flex items-center gap-2">
              <span :class="['rounded px-2 py-0.5 text-[10px] font-bold', permission === 'allow' ? 'bg-emerald-500/20 text-emerald-400' : 'bg-red-500/20 text-red-400']">
                {{ permission.toUpperCase() }}
              </span>
              <span class="text-surface-400">ACL</span>
            </div>
            <div class="space-y-1 text-surface-300">
              <div><span class="text-surface-500">Principal:</span> <span class="text-brand-400">{{ principal || 'User:?' }}</span></div>
              <div><span class="text-surface-500">Host:</span> <span class="text-surface-200">{{ host || '*' }}</span></div>
              <div><span class="text-surface-500">Resource:</span> <span class="text-blue-400">{{ resourceType }}</span><span class="text-surface-400">{{ resourceType !== 'cluster' ? `:${resourceName || '?'}` : ':kafka-cluster' }}</span></div>
              <div><span class="text-surface-500">Pattern:</span> <span class="text-surface-300">{{ patternType }}</span></div>
              <div><span class="text-surface-500">Operation:</span> <span class="text-yellow-400">{{ operation.toUpperCase() }}</span></div>
            </div>
          </div>
          <AppButton variant="primary" class="w-full" :disabled="submitting" @click="submit">
            {{ submitting ? "Creating…" : "Create ACL" }}
          </AppButton>
        </AppCard>

        <!-- Quick reference -->
        <AppCard class="space-y-3">
          <h2 class="section-title">Reference</h2>
          <div class="space-y-3 text-xs text-surface-400">
            <div>
              <div class="mb-1 font-semibold text-surface-300">Resource types</div>
              <div class="space-y-1">
                <div><span class="font-mono text-brand-400">Topic</span> — read/write Kafka topics</div>
                <div><span class="font-mono text-brand-400">Group</span> — manage consumer groups</div>
                <div><span class="font-mono text-brand-400">Cluster</span> — cluster-level admin operations</div>
                <div><span class="font-mono text-brand-400">Txn ID</span> — transactional producers</div>
              </div>
            </div>
            <hr class="border-surface-700/50" />
            <div>
              <div class="mb-1 font-semibold text-surface-300">Pattern types</div>
              <div class="space-y-1">
                <div><span class="font-mono text-brand-400">LITERAL</span> — exact name match</div>
                <div><span class="font-mono text-brand-400">PREFIXED</span> — all names starting with prefix</div>
                <div><span class="font-mono text-brand-400">MATCH</span> — wildcard / regex matching</div>
              </div>
            </div>
            <hr class="border-surface-700/50" />
            <div>
              <div class="mb-1 font-semibold text-surface-300">Common patterns</div>
              <div class="space-y-1">
                <div>Read-only consumer: <code class="font-mono">READ</code> on topic + <code class="font-mono">READ</code> on group</div>
                <div>Producer: <code class="font-mono">WRITE</code> + <code class="font-mono">CREATE</code> on topic</div>
                <div>Admin: <code class="font-mono">ALL</code> on cluster</div>
              </div>
            </div>
          </div>
        </AppCard>
      </div>
    </div>
  </div>
</template>
