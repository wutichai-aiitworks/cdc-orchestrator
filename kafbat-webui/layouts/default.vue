<script setup lang="ts">
import {
  Bell,
  Cable,
  ChevronLeft,
  Columns2,
  ArrowRightLeft,
  Copy,
  Database,
  FileJson,
  FileText,
  Gauge,
  GitBranch,
  HardDrive,
  Home,
  LayoutDashboard,
  List,
  PanelLeft,
  ScrollText,
  Server,
  Shield,
  Users,
  Workflow,
  X,
  LogOut,
} from "@lucide/vue"
import { isSsoAuth } from "~/utils/authMode"

const route = useRoute()
const cfg = useRuntimeConfig()
const { user, logout } = useAuth()
const ssoEnabled = computed(() => isSsoAuth(cfg))
const {
  data: capability,
  loading: capabilityLoading,
  error: capabilityError,
  refresh: refreshCapability,
} = useCdcCapability()
const sidebarOpen = ref(true)
const mobileMenuOpen = ref(false)
const mounted = ref(false)
let capabilityTimer: ReturnType<typeof setInterval> | null = null

const navGroups = [
  {
    label: "CDC",
    items: [
      { label: "Overview", to: "/", icon: Home },
      { label: "CDC Pipelines", to: "/pipelines", icon: Workflow },
      { label: "Schema Compare", to: "/schema-compare", icon: Columns2 },
      { label: "Event Processing", to: "/processing", icon: GitBranch },
    ],
  },
  {
    label: "Copy & Sync",
    items: [
      { label: "Postgres copy", to: "/pgcopy", icon: Copy },
      { label: "Load to Postgres", to: "/pgloader", icon: Database },
      { label: "Mongo copy", to: "/mongocopy", icon: Database },
      { label: "Redis sync", to: "/redisshake", icon: ArrowRightLeft },
    ],
  },
  {
    label: "Kafka Cluster",
    items: [
      { label: "Dashboard", to: "/dashboard", icon: LayoutDashboard },
      { label: "Brokers", to: "/brokers", icon: HardDrive },
      { label: "Topics", to: "/topics", icon: List },
      { label: "Consumers", to: "/consumer-groups", icon: Users },
      { label: "Schema Registry", to: "/schemas", icon: FileJson },
      { label: "Kafka Connect", to: "/connectors", icon: Cable },
      { label: "KSQL DB", to: "/ksqldb", icon: Database },
      { label: "ACL", to: "/acl", icon: Shield },
    ],
  },
  {
    label: "Operations",
    items: [
      { label: "Infrastructure", to: "/infrastructure", icon: Server },
      { label: "Monitoring", to: "/monitoring", icon: Gauge },
      { label: "Logs", to: "/logs", icon: ScrollText },
      { label: "Alerts", to: "/alerts", icon: Bell },
    ],
  },
  {
    label: "Developer",
    items: [{ label: "API Reference", to: "/api-docs", icon: FileText }],
  },
]
const nav = navGroups.flatMap((group) => group.items)

const isActive = (to: string) => {
  if (to === "/") return route.path === "/"
  return route.path.startsWith(to)
}

const pageTitle = computed(
  () => [...nav].sort((a, b) => b.to.length - a.to.length).find((item) => isActive(item.to))?.label || "Hyper CDC"
)

const orchestratorChip = computed(() => {
  if (capabilityLoading.value || (!capability.value && !capabilityError.value)) {
    return { status: "checking", label: "Checking Status" }
  }
  if (capabilityError.value) {
    return { status: "offline", label: "Orchestrator Offline" }
  }
  return { status: "online", label: "Orchestrator Online" }
})

const sidebarLabelClass = computed(() =>
  [
    "whitespace-nowrap transition-[opacity,transform] ease-out will-change-[opacity,transform]",
    sidebarOpen.value
      ? "opacity-100 translate-x-0 duration-200 delay-100"
      : "opacity-0 -translate-x-1 duration-100 pointer-events-none",
  ].join(" ")
)

watch(
  () => route.fullPath,
  () => {
    mobileMenuOpen.value = false
  }
)

onMounted(() => {
  mounted.value = true
  const stored = localStorage.getItem("cdc-sidebar-expanded")
  sidebarOpen.value = stored == null ? true : stored === "true"
  refreshCapability()
  capabilityTimer = setInterval(refreshCapability, 30000)
})

watch(sidebarOpen, (value) => {
  if (import.meta.client) localStorage.setItem("cdc-sidebar-expanded", String(value))
})

onUnmounted(() => {
  if (capabilityTimer) clearInterval(capabilityTimer)
})
</script>

<template>
  <div class="flex h-screen overflow-hidden bg-background text-foreground">
    <aside
      :class="[
        'hidden h-full flex-shrink-0 flex-col overflow-hidden border-r border-sidebar-border bg-sidebar text-sidebar-foreground md:flex',
        mounted ? 'transition-[width] duration-300 ease-in-out' : '',
        sidebarOpen ? 'w-56' : 'w-16',
      ]"
    >
      <div class="flex h-14 flex-shrink-0 items-center overflow-hidden border-b border-sidebar-border">
        <div class="flex h-14 w-16 flex-shrink-0 items-center justify-center">
          <img src="/favicon.png" alt="Hyper CDC" class="h-8 w-8 flex-shrink-0 object-contain" />
        </div>
        <span :class="['pr-3 text-sm font-bold text-sidebar-foreground', sidebarLabelClass]">
          Hyper CDC
        </span>
      </div>

      <nav class="min-h-0 flex-1 overflow-y-auto overflow-x-hidden px-2 py-4">
        <div v-for="(group, groupIndex) in navGroups" :key="group.label" :class="groupIndex ? 'mt-4' : ''">
          <p
            v-show="sidebarOpen"
            class="mb-1 px-3 text-sm font-medium text-muted-foreground"
            :class="sidebarLabelClass"
          >
            {{ group.label }}
          </p>
          <div class="space-y-1">
            <NuxtLink
              v-for="item in group.items"
              :key="item.to"
              :to="item.to"
              :title="sidebarOpen ? undefined : item.label"
              :class="[
                'flex h-10 items-center overflow-hidden rounded-xl text-sm font-medium transition-colors duration-150',
                isActive(item.to)
                  ? 'bg-brand-500/15 text-primary'
                  : 'text-muted-foreground hover:bg-sidebar-accent hover:text-sidebar-accent-foreground',
              ]"
            >
              <span class="flex h-10 w-12 flex-shrink-0 items-center justify-center">
                <component :is="item.icon" class="h-5 w-5" :stroke-width="1.75" />
              </span>
              <span :class="sidebarLabelClass">{{ item.label }}</span>
            </NuxtLink>
          </div>
        </div>
      </nav>

      <div class="flex-shrink-0 border-t border-sidebar-border px-2 py-4">
        <button
          type="button"
          class="flex h-10 w-full items-center overflow-hidden rounded-xl text-sm font-medium text-muted-foreground transition-colors hover:bg-sidebar-accent hover:text-sidebar-accent-foreground"
          :title="sidebarOpen ? 'Collapse' : 'Expand'"
          @click="sidebarOpen = !sidebarOpen"
        >
          <span class="flex h-10 w-12 flex-shrink-0 items-center justify-center">
            <ChevronLeft
              class="h-5 w-5 transition-transform duration-300"
              :class="sidebarOpen ? 'rotate-0' : 'rotate-180'"
              :stroke-width="1.75"
            />
          </span>
          <span :class="sidebarLabelClass">Collapse</span>
        </button>
      </div>
    </aside>

    <Transition name="fade">
      <div v-if="mobileMenuOpen" class="fixed inset-0 z-50 md:hidden">
        <button
          type="button"
          class="absolute inset-0 bg-black/60"
          aria-label="Close navigation"
          @click="mobileMenuOpen = false"
        />
        <aside class="relative flex h-full w-72 flex-col border-r border-sidebar-border bg-sidebar text-sidebar-foreground">
          <div class="flex h-14 items-center justify-between border-b border-sidebar-border px-4">
            <div class="flex items-center gap-3">
              <img src="/favicon.png" alt="Hyper CDC" class="h-8 w-8 object-contain" />
              <span class="text-sm font-bold">Hyper CDC</span>
            </div>
            <button type="button" class="icon-button" title="Close navigation" @click="mobileMenuOpen = false">
              <X class="h-4 w-4" :stroke-width="1.75" />
            </button>
          </div>
          <nav class="flex-1 overflow-y-auto px-3 py-4">
            <div v-for="(group, groupIndex) in navGroups" :key="group.label" :class="groupIndex ? 'mt-4' : ''">
              <p class="mb-1 px-3 text-sm font-medium text-muted-foreground">{{ group.label }}</p>
              <NuxtLink
                v-for="item in group.items"
                :key="item.to"
                :to="item.to"
                :class="[
                  'flex h-10 items-center gap-3 rounded-xl px-3 text-sm font-medium transition-colors',
                  isActive(item.to)
                    ? 'bg-brand-500/15 text-primary'
                    : 'text-muted-foreground hover:bg-sidebar-accent hover:text-sidebar-accent-foreground',
                ]"
              >
                <component :is="item.icon" class="h-5 w-5" :stroke-width="1.75" />
                {{ item.label }}
              </NuxtLink>
            </div>
          </nav>
        </aside>
      </div>
    </Transition>

    <div class="flex min-w-0 flex-1 flex-col overflow-hidden">
      <header class="flex h-14 flex-shrink-0 items-center justify-between border-b border-border bg-card px-4 md:px-6">
        <div class="flex min-w-0 items-center gap-3">
          <button
            type="button"
            class="icon-button md:hidden"
            title="Open navigation"
            aria-label="Open navigation"
            @click="mobileMenuOpen = true"
          >
            <PanelLeft class="h-4 w-4" :stroke-width="1.75" />
          </button>
          <h1 class="truncate text-lg font-semibold text-foreground">{{ pageTitle }}</h1>
        </div>

        <div class="flex items-center gap-2">
          <span class="hidden sm:inline-flex">
            <AppStatusBadge
              :status="orchestratorChip.status"
              :label="orchestratorChip.label"
            />
          </span>
          <AppThemeToggle />
          <template v-if="ssoEnabled">
            <span
              v-if="user?.email || user?.userId"
              class="hidden max-w-[12rem] truncate text-xs text-muted-foreground sm:inline"
              :title="user?.email || user?.userId"
            >
              {{ user?.email || user?.userId }}
            </span>
            <button
              type="button"
              class="icon-button"
              title="Sign out"
              aria-label="Sign out"
              @click="logout"
            >
              <LogOut class="h-4 w-4" :stroke-width="1.75" />
            </button>
          </template>
        </div>
      </header>

      <main class="flex-1 overflow-auto bg-background p-4 md:p-6">
        <slot />
      </main>
    </div>

    <AppToast />
  </div>
</template>

<style scoped>
.fade-enter-active,
.fade-leave-active {
  transition: opacity 0.2s ease;
}

.fade-enter-from,
.fade-leave-to {
  opacity: 0;
}
</style>
