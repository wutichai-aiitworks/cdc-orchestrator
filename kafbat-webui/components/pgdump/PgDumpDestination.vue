<script setup lang="ts">
// One SFTP server that receives the dump file. Read row (with host-key pin state) or edit form.
import { AlertTriangle, Lock, Pencil, Server, Trash2 } from "@lucide/vue"
import { PGDUMP_REDACTED, type PgDumpDestination } from "~/composables/usePgDump"

const d = defineModel<PgDumpDestination>({ required: true })
defineEmits<{ remove: [] }>()
const editing = ref(!d.value.host)

const saved = (v: string) => v === PGDUMP_REDACTED
const pinned = computed(() => !!d.value.host_key_fingerprint?.trim())
const authOptions = [
  { value: "key" as const, label: "Private key", help: "OpenSSH or PEM key. An encrypted key needs its passphrase." },
  { value: "password" as const, label: "Password" },
]
const authModel = computed({ get: () => d.value.auth, set: (v: "key" | "password") => (d.value.auth = v) })
const fingerprintOk = computed(() => !d.value.host_key_fingerprint || /^SHA256:[A-Za-z0-9+/]{43}=?$/.test(d.value.host_key_fingerprint.trim()))
</script>

<template>
  <div class="rounded-lg border border-border">
    <div class="flex items-center gap-3 p-3">
      <span class="flex h-8 w-8 shrink-0 items-center justify-center rounded-md border border-border bg-muted text-muted-foreground" aria-hidden="true">
        <Server class="h-4 w-4" :stroke-width="1.75" />
      </span>
      <div class="min-w-0 flex-1">
        <div class="truncate text-sm">
          <span class="font-medium text-foreground">{{ d.name || d.host || "New SFTP server" }}</span>
          <span v-if="d.host" class="text-muted-foreground"> {{ d.username || "?" }}@{{ d.host }}:{{ d.port || "22" }}</span>
        </div>
        <div class="truncate font-mono text-xs text-muted-foreground">{{ d.remote_dir || "home directory" }}</div>
      </div>
      <span
        v-if="d.host"
        class="hidden shrink-0 items-center gap-1 text-xs sm:inline-flex"
        :class="pinned ? 'text-emerald-600 dark:text-emerald-400' : 'text-amber-600 dark:text-amber-400'"
        :title="pinned ? d.host_key_fingerprint : 'Any host key is accepted. Pin the fingerprint shown after the first upload.'"
      >
        <component :is="pinned ? Lock : AlertTriangle" class="h-3.5 w-3.5" :stroke-width="2" />
        {{ pinned ? "Host key pinned" : "Host key not pinned" }}
      </span>
      <AppButton size="sm" variant="ghost" :title="editing ? 'Close' : 'Edit server'" @click="editing = !editing">
        <Pencil class="h-3.5 w-3.5" :stroke-width="1.75" />{{ editing ? "Close" : "Edit" }}
      </AppButton>
      <AppButton size="icon" variant="ghost" title="Remove server" @click="$emit('remove')">
        <Trash2 class="h-4 w-4" :stroke-width="1.75" />
      </AppButton>
    </div>

    <div v-if="editing" class="grid gap-3 border-t border-border p-3 sm:grid-cols-6">
      <label class="sm:col-span-2">
        <span class="auth-label !mb-1.5">Name</span>
        <input v-model="d.name" class="auth-input" placeholder="backup-sg" />
      </label>
      <label class="sm:col-span-3">
        <span class="auth-label !mb-1.5">Host</span>
        <input v-model.trim="d.host" class="auth-input" placeholder="backup.example.com" spellcheck="false" />
      </label>
      <label class="sm:col-span-1">
        <span class="auth-label !mb-1.5">Port</span>
        <input v-model.trim="d.port" class="auth-input tabular-nums" inputmode="numeric" placeholder="22" />
      </label>
      <label class="sm:col-span-2">
        <span class="auth-label !mb-1.5">Username</span>
        <input v-model.trim="d.username" class="auth-input" placeholder="dumps" spellcheck="false" />
      </label>
      <label class="sm:col-span-4">
        <span class="auth-label !mb-1.5">Directory</span>
        <input v-model.trim="d.remote_dir" class="auth-input font-mono !text-xs" placeholder="/srv/dumps/shop (created if missing)" spellcheck="false" />
      </label>

      <div class="sm:col-span-6">
        <span class="auth-label !mb-1.5">Sign in with</span>
        <TransferSegmented v-model="authModel" :options="authOptions" label="SFTP authentication" />
      </div>
      <template v-if="d.auth === 'key'">
        <div class="sm:col-span-6">
          <span class="auth-label !mb-1.5">Private key</span>
          <div v-if="saved(d.private_key)" class="flex h-10 items-center justify-between rounded-xl border border-input px-4 text-sm">
            <span class="text-muted-foreground">Saved</span>
            <button type="button" class="btn-ghost text-xs" @click="d.private_key = ''">Replace</button>
          </div>
          <textarea
            v-else
            v-model="d.private_key"
            rows="4"
            class="auth-input font-mono !text-xs"
            placeholder="-----BEGIN OPENSSH PRIVATE KEY-----"
            spellcheck="false"
            aria-label="Private key"
          />
        </div>
        <div class="sm:col-span-3">
          <span class="auth-label !mb-1.5">Key passphrase</span>
          <div v-if="saved(d.passphrase)" class="flex h-10 items-center justify-between rounded-xl border border-input px-4 text-sm">
            <span class="text-muted-foreground">Saved</span>
            <button type="button" class="btn-ghost text-xs" @click="d.passphrase = ''">Replace</button>
          </div>
          <input v-else v-model="d.passphrase" type="password" class="auth-input" placeholder="Only for an encrypted key" autocomplete="new-password" aria-label="Key passphrase" />
        </div>
      </template>
      <div v-else class="sm:col-span-3">
        <span class="auth-label !mb-1.5">Password</span>
        <div v-if="saved(d.password)" class="flex h-10 items-center justify-between rounded-xl border border-input px-4 text-sm">
          <span class="text-muted-foreground">Saved</span>
          <button type="button" class="btn-ghost text-xs" @click="d.password = ''">Replace</button>
        </div>
        <input v-else v-model="d.password" type="password" class="auth-input" autocomplete="new-password" aria-label="SFTP password" />
      </div>
      <label class="sm:col-span-6">
        <span class="auth-label !mb-1.5">Host key fingerprint</span>
        <input
          v-model.trim="d.host_key_fingerprint"
          class="auth-input font-mono !text-xs"
          :class="fingerprintOk ? '' : '!border-red-500'"
          placeholder="SHA256:… (leave empty to learn it from the first upload)"
          spellcheck="false"
        />
        <span class="mt-1 block text-xs" :class="fingerprintOk ? 'text-muted-foreground' : 'text-red-600 dark:text-red-400'">
          {{ fingerprintOk ? "When set, an upload to a server with a different key is refused." : "Use the OpenSSH form: SHA256: followed by 43 characters." }}
        </span>
      </label>
    </div>
  </div>
</template>
