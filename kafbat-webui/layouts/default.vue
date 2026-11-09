<script setup lang="ts">
import {
  Bell,
  Cable,
  ChevronLeft,
  ChevronRight,
  Columns2,
  ArrowRightLeft,
  Copy,
  Database,
  DatabaseZap,
  FileJson,
  FileText,
  Gauge,
  GitBranch,
  HardDrive,
  FolderSync,
  HardDriveDownload,
  Import,
  Home,
  LayoutDashboard,
  Network,
  List,
  PanelLeft,
  ScrollText,
  Server,
  Shield,
  Users,
  Workflow,
  X,
} from "@lucide/vue"
import type { Component } from "vue"
import { isAuthDisabled } from "~/utils/authMode"

const route = useRoute()
const { ribbon: modeRibbon } = useAppMode()
const cfg = useRuntimeConfig()
const authEnabled = computed(() => !isAuthDisabled(cfg))
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

type NavItem = { label: string; icon: Component; to?: string; feature?: string; children?: NavItem[] }
type NavGroup = { label: string; items: NavItem[]; /** Folds under its header (closed by default). */ collapsible?: boolean }

const navGroups: NavGroup[] = [
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
    collapsible: true,
    items: [
      {
        label: "Postgres tools",
        icon: DatabaseZap,
        children: [
          { label: "Postgres copy", to: "/pgcopy", icon: Copy },
          { label: "Postgres dump", to: "/pgdump", icon: HardDriveDownload },
          { label: "Load to Postgres", to: "/pgloader", icon: Import },
        ],
      },
      { label: "Mongo copy", to: "/mongocopy", icon: Database },
      { label: "Redis sync", to: "/redisshake", icon: ArrowRightLeft },
      { label: "File sync", to: "/rsync", icon: FolderSync },
      { label: "WireGuard tunnels", to: "/tunnels", icon: Network },
    ],
  },
  {
    label: "Kafka Cluster",
    collapsible: true,
    items: [
      { label: "Dashboard", to: "/dashboard", icon: LayoutDashboard },
      { label: "Brokers", to: "/brokers", icon: HardDrive },
      { label: "Topics", to: "/topics", icon: List },
      { label: "Consumers", to: "/consumer-groups", icon: Users },
      { label: "Schema Registry", to: "/schemas", icon: FileJson },
      { label: "Kafka Connect", to: "/connectors", icon: Cable },
      { label: "KSQL DB", to: "/ksqldb", icon: Database, feature: "ksql_enabled" },
      { label: "ACL", to: "/acl", icon: Shield },
    ],
  },
  {
    label: "Operations",
    collapsible: true,
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
/** Every page link, children included (page title lookup). */
const nav = navGroups
  .flatMap((group) => group.items.flatMap((item) => item.children ?? [item]))
  .filter((item): item is NavItem & { to: string } => !!item.to)
// Items tied to a feature switch (e.g. ORCH_KSQL_ENABLED) are hidden once the orchestrator reports
// it off; shown while the capability is still loading so the enabled default does not flicker.
const featureOn = (item: NavItem) => !item.feature || capability.value?.features?.[item.feature] !== false
const visibleNavGroups = computed(() =>
  navGroups
    .map((group) => ({
      ...group,
      items: group.items
        .filter(featureOn)
        .map((item) => (item.children ? { ...item, children: item.children.filter(featureOn) } : item))
        .filter((item) => !item.children || item.children.length),
    }))
    .filter((group) => group.items.length),
)

// Parent items (e.g. "Postgres tools") are collapsed by default and open only when clicked; the
// choice is remembered per browser. Being on one of their pages does not force them open (that
// also made them impossible to close) — the parent row is highlighted instead.
// "-v2": earlier builds remembered parents forced open; the new key starts everyone collapsed once.
const NAV_OPEN_KEY = "cdc-nav-open-v2"
const navOpen = ref<Record<string, boolean>>({})
const isParentActive = (item: NavItem) => !!item.children?.some((c) => c.to && isActive(c.to))
const isNavOpen = (item: NavItem) => navOpen.value[item.label] === true
const toggleNav = (item: NavItem) => {
  navOpen.value = { ...navOpen.value, [item.label]: !isNavOpen(item) }
  localStorage.setItem(NAV_OPEN_KEY, JSON.stringify(navOpen.value))
}
// Menu groups (Copy & Sync, Kafka Cluster, Operations) fold under their header; closed by default,
// remembered per browser. Opening a page inside a group opens that group so the page stays visible.
const NAV_GROUPS_KEY = "cdc-nav-groups"
const groupOpen = ref<Record<string, boolean>>({})
const isGroupOpen = (g: NavGroup) => !g.collapsible || groupOpen.value[g.label] === true
const setGroupOpen = (label: string, open: boolean) => {
  groupOpen.value = { ...groupOpen.value, [label]: open }
  localStorage.setItem(NAV_GROUPS_KEY, JSON.stringify(groupOpen.value))
}
const toggleGroup = (g: NavGroup) => setGroupOpen(g.label, !isGroupOpen(g))
const leafCount = (g: NavGroup) => g.items.reduce((n, i) => n + (i.children ? i.children.length : 1), 0)
const groupHasActive = (g: NavGroup) => g.items.some((i) => (i.children ?? [i]).some((l) => l.to && isActive(l.to)))
const openActiveGroup = () => {
  for (const g of navGroups) if (g.collapsible && groupHasActive(g) && !isGroupOpen(g)) setGroupOpen(g.label, true)
}

/** The collapsed icon rail has no room for a parent row: its children become plain icons. */
const railItems = (items: NavItem[]) => (sidebarOpen.value ? items : items.flatMap((item) => item.children ?? [item]))

const isActive = (to: string) => {
  if (to === "/") return route.path === "/"
  return route.path.startsWith(to)
}

const { reselect } = useNavReselect()
// Same-route clicks don't navigate; tell the page so it can return to its landing view.
const onNavClick = (to: string) => {
  if (route.path === to) reselect(to)
  mobileMenuOpen.value = false
}

const pageTitle = computed(
  () => [...nav].sort((a, b) => b.to.length - a.to.length).find((item) => isActive(item.to))?.label || "Hyper CDC"
)
/** Breadcrumb trail for the top bar: menu group, parent item (e.g. Postgres tools), page. */
const breadcrumb = computed(() => {
  for (const group of navGroups) {
    for (const item of group.items) {
      const leaves = item.children ?? [item]
      const hit = [...leaves].sort((a, b) => (b.to?.length || 0) - (a.to?.length || 0)).find((l) => l.to && isActive(l.to))
      if (hit) return [group.label, ...(item.children ? [item.label] : []), hit.label]
    }
  }
  return [pageTitle.value]
})

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
    if (mounted.value) openActiveGroup()
  }
)

onMounted(() => {
  mounted.value = true
  try {
    navOpen.value = JSON.parse(localStorage.getItem(NAV_OPEN_KEY) || "{}")
  } catch {
    navOpen.value = {}
  }
  try {
    groupOpen.value = JSON.parse(localStorage.getItem(NAV_GROUPS_KEY) || "{}")
  } catch {
    groupOpen.value = {}
  }
  openActiveGroup()
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
        <div v-for="(group, groupIndex) in visibleNavGroups" :key="group.label" :class="groupIndex ? 'mt-4' : ''">
          <button
            v-if="group.collapsible"
            v-show="sidebarOpen"
            type="button"
            class="mb-1 flex w-full items-center gap-1.5 rounded-lg px-3 py-0.5 text-left text-sm font-medium text-muted-foreground transition-colors hover:text-foreground"
            :class="sidebarLabelClass"
            :aria-expanded="isGroupOpen(group)"
            @click="toggleGroup(group)"
          >
            <span class="flex-1 truncate">{{ group.label }}</span>
            <span v-if="!isGroupOpen(group)" class="rounded bg-muted px-1.5 text-[11px] tabular-nums">{{ leafCount(group) }}</span>
            <ChevronRight class="h-3.5 w-3.5 shrink-0 transition-transform duration-200" :class="isGroupOpen(group) ? 'rotate-90' : ''" :stroke-width="2" />
          </button>
          <p
            v-else
            v-show="sidebarOpen"
            class="mb-1 px-3 text-sm font-medium text-muted-foreground"
            :class="sidebarLabelClass"
          >
            {{ group.label }}
          </p>
          <!-- the icon rail (sidebar minimised) always shows every page -->
          <div v-show="!sidebarOpen || isGroupOpen(group)" class="space-y-1">
            <template v-for="item in railItems(group.items)" :key="item.to || item.label">
              <template v-if="item.children">
                <button
                  type="button"
                  :aria-expanded="isNavOpen(item)"
                  @click="toggleNav(item)"
                  :class="[
                    'flex h-10 w-full items-center overflow-hidden rounded-xl text-sm font-medium transition-colors duration-150',
                    isParentActive(item)
                      ? isNavOpen(item)
                        ? 'text-primary hover:bg-sidebar-accent'
                        : 'bg-brand-500/15 text-primary'
                      : 'text-muted-foreground hover:bg-sidebar-accent hover:text-sidebar-accent-foreground',
                  ]"
                >
                  <span class="flex h-10 w-12 flex-shrink-0 items-center justify-center">
                    <component :is="item.icon" class="h-5 w-5" :stroke-width="1.75" />
                  </span>
                  <span class="flex-1 text-left" :class="sidebarLabelClass">{{ item.label }}</span>
                  <ChevronRight
                    class="mr-3 h-4 w-4 flex-shrink-0 transition-transform duration-200"
                    :class="isNavOpen(item) ? 'rotate-90' : ''"
                    :stroke-width="1.75"
                  />
                </button>
                <div v-show="isNavOpen(item)" class="ml-6 space-y-1 border-l border-sidebar-border pl-2">
                  <NuxtLink
                    v-for="child in item.children"
                    :key="child.to"
                    :to="child.to"
                    @click="onNavClick(child.to!)"
                    :class="[
                      'flex h-9 items-center gap-3 overflow-hidden whitespace-nowrap rounded-lg px-3 text-sm font-medium transition-colors duration-150',
                      isActive(child.to!)
                        ? 'bg-brand-500/15 text-primary'
                        : 'text-muted-foreground hover:bg-sidebar-accent hover:text-sidebar-accent-foreground',
                    ]"
                  >
                    <component :is="child.icon" class="h-4 w-4 flex-shrink-0" :stroke-width="1.75" />
                    {{ child.label }}
                  </NuxtLink>
                </div>
              </template>
              <NuxtLink
                v-else
                :to="item.to!"
                :title="sidebarOpen ? undefined : item.label"
                @click="onNavClick(item.to!)"
                :class="[
                  'flex h-10 items-center overflow-hidden rounded-xl text-sm font-medium transition-colors duration-150',
                  isActive(item.to!)
                    ? 'bg-brand-500/15 text-primary'
                    : 'text-muted-foreground hover:bg-sidebar-accent hover:text-sidebar-accent-foreground',
                ]"
              >
                <span class="flex h-10 w-12 flex-shrink-0 items-center justify-center">
                  <component :is="item.icon" class="h-5 w-5" :stroke-width="1.75" />
                </span>
                <span :class="sidebarLabelClass">{{ item.label }}</span>
              </NuxtLink>
            </template>
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
            <div v-for="(group, groupIndex) in visibleNavGroups" :key="group.label" :class="groupIndex ? 'mt-4' : ''">
              <button
                v-if="group.collapsible"
                type="button"
                class="mb-1 flex w-full items-center gap-1.5 rounded-lg px-3 py-0.5 text-left text-sm font-medium text-muted-foreground hover:text-foreground"
                :aria-expanded="isGroupOpen(group)"
                @click="toggleGroup(group)"
              >
                <span class="flex-1">{{ group.label }}</span>
                <span v-if="!isGroupOpen(group)" class="rounded bg-muted px-1.5 text-[11px] tabular-nums">{{ leafCount(group) }}</span>
                <ChevronRight class="h-3.5 w-3.5 transition-transform duration-200" :class="isGroupOpen(group) ? 'rotate-90' : ''" :stroke-width="2" />
              </button>
              <p v-else class="mb-1 px-3 text-sm font-medium text-muted-foreground">{{ group.label }}</p>
              <template v-for="item in isGroupOpen(group) ? group.items : []" :key="item.to || item.label">
                <template v-if="item.children">
                  <button
                    type="button"
                    :aria-expanded="isNavOpen(item)"
                    @click="toggleNav(item)"
                    :class="[
                      'flex h-10 w-full items-center gap-3 rounded-xl px-3 text-sm font-medium transition-colors',
                      isParentActive(item)
                        ? isNavOpen(item)
                          ? 'text-primary hover:bg-sidebar-accent'
                          : 'bg-brand-500/15 text-primary'
                        : 'text-muted-foreground hover:bg-sidebar-accent hover:text-sidebar-accent-foreground',
                    ]"
                  >
                    <component :is="item.icon" class="h-5 w-5" :stroke-width="1.75" />
                    <span class="flex-1 text-left">{{ item.label }}</span>
                    <ChevronRight class="h-4 w-4 transition-transform duration-200" :class="isNavOpen(item) ? 'rotate-90' : ''" :stroke-width="1.75" />
                  </button>
                  <div v-show="isNavOpen(item)" class="ml-5 space-y-1 border-l border-sidebar-border pl-2">
                    <NuxtLink
                      v-for="child in item.children"
                      :key="child.to"
                      :to="child.to"
                      @click="onNavClick(child.to!)"
                      :class="[
                        'flex h-9 items-center gap-3 rounded-lg px-3 text-sm font-medium transition-colors',
                        isActive(child.to!)
                          ? 'bg-brand-500/15 text-primary'
                          : 'text-muted-foreground hover:bg-sidebar-accent hover:text-sidebar-accent-foreground',
                      ]"
                    >
                      <component :is="child.icon" class="h-4 w-4" :stroke-width="1.75" />
                      {{ child.label }}
                    </NuxtLink>
                  </div>
                </template>
                <NuxtLink
                  v-else
                  :to="item.to!"
                  @click="onNavClick(item.to!)"
                  :class="[
                    'flex h-10 items-center gap-3 rounded-xl px-3 text-sm font-medium transition-colors',
                    isActive(item.to!)
                      ? 'bg-brand-500/15 text-primary'
                      : 'text-muted-foreground hover:bg-sidebar-accent hover:text-sidebar-accent-foreground',
                  ]"
                >
                  <component :is="item.icon" class="h-5 w-5" :stroke-width="1.75" />
                  {{ item.label }}
                </NuxtLink>
              </template>
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
          <nav aria-label="Breadcrumb" class="min-w-0">
            <ol class="flex min-w-0 items-center gap-1.5 text-sm">
              <li v-for="(crumb, i) in breadcrumb" :key="i" class="flex min-w-0 items-center gap-1.5">
                <span v-if="i" class="text-muted-foreground opacity-50" aria-hidden="true">/</span>
                <span
                  class="truncate"
                  :class="i === breadcrumb.length - 1 ? 'font-medium text-foreground' : 'text-muted-foreground'"
                  :aria-current="i === breadcrumb.length - 1 ? 'page' : undefined"
                >{{ crumb }}</span>
              </li>
            </ol>
          </nav>
        </div>

        <!-- With an APP_MODE ribbon in the corner (Maintenance / Enhanced), keep the theme button and avatar clear of it. -->
        <div class="flex items-center gap-2" :class="modeRibbon ? 'mr-16 md:mr-14' : ''">
          <span class="hidden sm:inline-flex">
            <AppStatusBadge
              :status="orchestratorChip.status"
              :label="orchestratorChip.label"
            />
          </span>
          <AppThemeToggle />
          <AppUserMenu v-if="authEnabled" />
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
