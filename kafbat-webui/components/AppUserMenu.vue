<script setup lang="ts">
import { LogOut } from "@lucide/vue"
import { onClickOutside, useEventListener } from "@vueuse/core"

const { user, logout } = useAuth()

const open = ref(false)
const root = ref<HTMLElement | null>(null)
const panel = ref<HTMLElement | null>(null)
const pos = ref({ top: 0, right: 0 })

const displayName = computed(() => user.value?.email || user.value?.userId || "User")

const initials = computed(() => {
  const base = displayName.value.split("@")[0] || ""
  const parts = base.split(/[\s._-]+/).filter(Boolean)
  const letters = parts.length > 1 ? parts[0][0] + parts[1][0] : base.slice(0, 2)
  return (letters || "U").toUpperCase()
})

const providerLabel = computed(() => {
  const provider = user.value?.authProvider
  if (provider === "local") return "Local account"
  if (provider === "authelia") return "Authelia SSO"
  return "SSO account"
})

const place = () => {
  if (!open.value || !root.value) return
  const r = root.value.getBoundingClientRect()
  pos.value = { top: r.bottom + 8, right: Math.max(8, window.innerWidth - r.right) }
}

const toggle = () => {
  open.value = !open.value
  if (open.value) nextTick(place)
}

const signOut = async () => {
  open.value = false
  await logout()
}

onClickOutside(root, () => (open.value = false), { ignore: [panel] })

useEventListener(window, "scroll", place, { capture: true, passive: true })
useEventListener(window, "resize", place)
useEventListener(document, "keydown", (event: KeyboardEvent) => {
  if (event.key === "Escape") open.value = false
})
</script>

<template>
  <div ref="root" class="relative inline-flex">
    <button
      type="button"
      class="avatar"
      :title="displayName"
      aria-label="Account menu"
      aria-haspopup="menu"
      :aria-expanded="open"
      @click="toggle"
    >
      {{ initials }}
    </button>
    <Teleport to="body">
      <div
        v-if="open"
        ref="panel"
        role="menu"
        class="menu-panel w-60"
        :style="{ top: `${pos.top}px`, right: `${pos.right}px` }"
      >
        <div class="flex items-center gap-3 px-3 py-2.5">
          <span class="avatar">{{ initials }}</span>
          <div class="min-w-0">
            <p class="truncate text-sm font-medium text-foreground" :title="displayName">{{ displayName }}</p>
            <p class="truncate text-xs text-muted-foreground">{{ providerLabel }}</p>
          </div>
        </div>
        <div class="my-1 h-px bg-border" />
        <button type="button" role="menuitem" class="menu-item" @click="signOut">
          <LogOut class="h-4 w-4" :stroke-width="1.75" />
          Sign out
        </button>
      </div>
    </Teleport>
  </div>
</template>
