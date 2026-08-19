<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import type { Component } from 'vue'
import { ref, computed, watch, onMounted } from 'vue'
import { useRoute } from 'vue-router'
import { useAuthStore } from '../stores/auth'
import { useUiStore } from '../stores/ui'
import { useTheme } from '../composables/useTheme'
import { useTimezone } from '../composables/useTimezone'
import BackendUnreachable from '../components/BackendUnreachable.vue'
import {
  Activity,
  Bell,
  BookOpen,
  Braces,
  Cable,
  ChevronLeft,
  ChevronRight,
  Clock,
  Database,
  HardDrive,
  KeyRound,
  LayoutGrid,
  Lock,
  LogOut,
  Menu,
  Server,
  Settings,
  SlidersHorizontal,
  Users,
  Wrench,
} from '@lucide/vue'

const iconMap: Record<string, Component> = {
  activity: Activity,
  notifications: Bell,
  dashboard: LayoutGrid,
  excludes: SlidersHorizontal,
  groups: Users,
  hosts: Server,
  repos: Database,
  roles: Lock,
  schedules: Clock,
  'server-quotas': HardDrive,
  system: Wrench,
  tokens: KeyRound,
  tunnels: Cable,
  users: Users,
}

const authStore = useAuthStore()
const uiStore = useUiStore()
const route = useRoute()

const ROOT_PATH = '/'

const isAdmin = computed(() => authStore.isAdmin)
const isFullWidth = computed(() => route.path.startsWith('/activity'))
const isDashboard = computed(() => route.path === ROOT_PATH)
const settingsOpen = ref(false)

const mainNav = [
  { to: '/', label: 'Dashboard', icon: 'dashboard' },
  { to: '/agents', label: 'Agents', icon: 'hosts' },
  { to: '/repos', label: 'Repos', icon: 'repos' },
  { to: '/schedules', label: 'Schedules', icon: 'schedules' },
  { to: '/tunnels', label: 'Tunnels', icon: 'tunnels' },
  { to: '/notifications', label: 'Notifications', icon: 'notifications' },
  { to: '/activity', label: 'Activity', icon: 'activity' },
]

const adminNav: NavItem[] = []

interface NavItem {
  to: string
  label: string
  icon: string
}

interface NavGroup {
  label: string | null
  items: NavItem[]
}

const settingsNav = computed((): NavGroup[] => {
  const groups: NavGroup[] = [
    {
      label: null,
      items: [
        { to: '/excludes', label: 'Excludes', icon: 'excludes' },
        { to: '/profile', label: 'Profile', icon: 'tokens' },
      ],
    },
  ]
  if (isAdmin.value) {
    groups[0].items.push({ to: '/system', label: 'System', icon: 'system' })
    groups[0].items.push({
      to: '/server-quotas',
      label: 'Server Quotas',
      icon: 'server-quotas',
    })
    groups.push({
      label: 'Access Control',
      items: [
        { to: '/users', label: 'Users', icon: 'users' },
        { to: '/admin/groups', label: 'Groups', icon: 'groups' },
        { to: '/admin/roles', label: 'Roles', icon: 'roles' },
        { to: '/audit-log', label: 'Audit Log', icon: 'activity' },
      ],
    })
  }
  return groups
})

watch(
  () => route.path,
  () => {
    uiStore.closeMobileSidebar()
  },
)

const { loadFromBackend: loadTheme } = useTheme()
const { loadFromBackend: loadTimezone } = useTimezone()
onMounted(() => {
  loadTheme()
  loadTimezone()
})
</script>

<template>
  <div
    class="shell"
    :class="{ collapsed: uiStore.sidebarCollapsed }"
  >
    <!-- Mobile header -->
    <header class="mobile-header">
      <button
        class="mobile-toggle"
        aria-label="Open menu"
        @click="uiStore.openMobileSidebar()"
      >
        <Menu :size="20" />
      </button>
      <span class="mobile-brand">Assimilate</span>
    </header>

    <!-- Backdrop for mobile -->
    <div
      v-if="uiStore.sidebarMobileOpen"
      class="backdrop"
      @click="uiStore.closeMobileSidebar()"
    />

    <aside
      class="sidebar"
      :class="{ 'mobile-open': uiStore.sidebarMobileOpen }"
    >
      <div class="brand">
        <img
          class="brand-icon"
          src="/icon.png"
          alt="Assimilate"
          width="80"
          height="80"
        />
        <span class="brand-text">Assimilate</span>
        <button
          class="collapse-toggle"
          :aria-label="uiStore.sidebarCollapsed ? 'Expand sidebar' : 'Collapse sidebar'"
          @click="uiStore.toggleSidebar()"
        >
          <component
            :is="uiStore.sidebarCollapsed ? ChevronRight : ChevronLeft"
            :size="14"
          />
        </button>
      </div>
      <nav class="nav">
        <RouterLink
          v-for="item in mainNav"
          :key="item.to"
          :to="item.to"
          class="nav-link"
          :title="uiStore.sidebarCollapsed ? item.label : undefined"
        >
          <component
            :is="iconMap[item.icon]"
            class="nav-icon"
            :size="16"
          />
          <span class="nav-label">{{ item.label }}</span>
        </RouterLink>

        <template v-if="isAdmin">
          <RouterLink
            v-for="item in adminNav"
            :key="item.to"
            :to="item.to"
            class="nav-link"
            :title="uiStore.sidebarCollapsed ? item.label : undefined"
          >
            <component
              :is="iconMap[item.icon]"
              class="nav-icon"
              :size="16"
            />
            <span class="nav-label">{{ item.label }}</span>
          </RouterLink>
        </template>

        <div class="nav-group">
          <button
            class="nav-group-toggle"
            :class="{ open: settingsOpen }"
            :title="uiStore.sidebarCollapsed ? 'Settings' : undefined"
            @click="settingsOpen = !settingsOpen"
          >
            <Settings
              class="nav-icon"
              :size="16"
            />
            <span class="nav-label">Settings</span>
            <ChevronRight
              class="nav-group-chevron"
              :size="14"
            />
          </button>
          <div
            v-show="settingsOpen"
            class="nav-group-items"
          >
            <template
              v-for="(group, gi) in settingsNav"
              :key="gi"
            >
              <span
                v-if="group.label && !uiStore.sidebarCollapsed"
                class="nav-subgroup-label"
              >
                {{ group.label }}
              </span>
              <RouterLink
                v-for="item in group.items"
                :key="item.to"
                :to="item.to"
                class="nav-link nav-link-nested"
                :title="uiStore.sidebarCollapsed ? item.label : undefined"
              >
                <component
                  :is="iconMap[item.icon]"
                  class="nav-icon"
                  :size="16"
                />
                <span class="nav-label">{{ item.label }}</span>
              </RouterLink>
            </template>
          </div>
        </div>
      </nav>
      <div class="sidebar-footer">
        <div
          v-if="authStore.user"
          class="user-info"
        >
          <span class="user-name">{{ authStore.user.username }}</span>
          <span
            class="user-role"
            :class="authStore.user.role"
            >{{ authStore.user.role }}</span
          >
        </div>
        <div class="sidebar-actions">
          <a
            href="/docs/"
            target="_blank"
            class="signout-btn docs-btn"
            title="Documentation"
          >
            <BookOpen
              class="nav-icon"
              :size="16"
            />
            <span class="nav-label">Docs</span>
          </a>
          <a
            href="/api/docs"
            target="_blank"
            class="signout-btn docs-btn"
            title="API reference"
          >
            <Braces
              class="nav-icon"
              :size="16"
            />
            <span class="nav-label">API</span>
          </a>
          <button
            v-if="authStore.user"
            class="signout-btn"
            title="Sign out"
            @click="authStore.logout()"
          >
            <LogOut
              class="nav-icon"
              :size="16"
            />
            <span class="nav-label">Sign out</span>
          </button>
        </div>
      </div>
    </aside>

    <main class="content">
      <div
        class="page-wrapper"
        :class="{
          'page-wrapper--dashboard': !isFullWidth && isDashboard,
          'page-wrapper--constrained': !isFullWidth && !isDashboard,
        }"
      >
        <RouterView v-slot="{ Component: RouteComponent }">
          <Transition name="page">
            <component
              :is="RouteComponent"
              :key="route.path"
            />
          </Transition>
        </RouterView>
      </div>
    </main>

    <BackendUnreachable />
  </div>
</template>

<style scoped>
.shell {
  display: flex;
  min-height: 100vh;
  background: var(--bg-base);
  color: var(--text-primary);
}

.sidebar {
  width: 210px;
  padding: var(--space-7);
  background: var(--bg-sidebar);
  border-right: 1px solid var(--border);
  display: flex;
  flex-direction: column;
  flex-shrink: 0;
  transition: width var(--duration-slow) ease;
  overflow: hidden;
  will-change: width;
  position: sticky;
  top: 0;
  height: 100vh;
}

.collapsed .sidebar {
  width: 72px;
  padding: var(--space-7) var(--space-4);
}

.brand {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--space-4);
  margin-bottom: var(--space-8);
  padding: 0 var(--space-4);
}

.brand-icon {
  flex-shrink: 0;
  border-radius: var(--radius-sm);
}

.brand-text {
  font-size: var(--fs-lg);
  font-weight: 700;
  color: var(--text-primary);
  white-space: nowrap;
  opacity: 1;
  transition: opacity var(--duration-base) ease;
}

.collapsed .brand-text {
  opacity: 0;
  width: 0;
  overflow: hidden;
}

.collapsed .brand {
  padding: 0;
}

.collapsed .brand-icon {
  width: 40px;
  height: 40px;
}

.collapse-toggle {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 1.5rem;
  height: 1.5rem;
  border: none;
  background: transparent;
  color: var(--text-muted);
  border-radius: var(--radius-sm);
  cursor: pointer;
  transition:
    color var(--duration-base),
    background var(--duration-base);
  flex-shrink: 0;
}

.collapse-toggle:hover {
  color: var(--text-primary);
  background: var(--bg-hover);
}

.collapsed .collapse-toggle {
  width: 1.5rem;
  height: 1.5rem;
}

.collapse-toggle svg {
  width: 14px;
  height: 14px;
}

.nav {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
  flex: 1;
  overflow-y: auto;
  min-height: 0;
}

.nav-icon {
  width: 16px;
  height: 16px;
  flex-shrink: 0;
}

.nav-label {
  white-space: nowrap;
  opacity: 1;
  transition: opacity var(--duration-base) ease;
}

.collapsed .nav-label {
  display: none;
}

.nav-link {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  padding: var(--space-4) var(--space-5);
  border-radius: var(--radius-sm);
  color: var(--text-secondary);
  text-decoration: none;
  font-size: var(--fs-base);
  font-weight: 500;
  transition:
    background var(--duration-base),
    color var(--duration-base);
  overflow: hidden;
}

.collapsed .nav-link {
  justify-content: center;
  padding: var(--space-4);
  gap: 0;
}

.nav-link:hover {
  background: var(--bg-hover);
  color: var(--text-primary);
}

.nav-link.router-link-active {
  background: var(--accent-subtle);
  color: var(--accent);
  font-weight: 600;
}

.nav-link-nested {
  padding-left: var(--space-9);
  font-size: var(--fs-sm);
}

.collapsed .nav-link-nested {
  padding-left: var(--space-4);
}

.nav-group {
  margin-top: var(--space-4);
}

.nav-group-toggle {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  width: 100%;
  padding: var(--space-4) var(--space-5);
  border-radius: var(--radius-sm);
  border: none;
  background: transparent;
  color: var(--text-secondary);
  font-size: var(--fs-base);
  font-weight: 500;
  cursor: pointer;
  transition:
    background var(--duration-base),
    color var(--duration-base);
  overflow: hidden;
}

.collapsed .nav-group-toggle {
  justify-content: center;
  padding: var(--space-4);
  gap: 0;
}

.nav-group-toggle:hover {
  background: var(--bg-hover);
  color: var(--text-primary);
}

.nav-group-toggle.open {
  color: var(--text-primary);
}

.nav-group-chevron {
  width: 14px;
  height: 14px;
  margin-left: auto;
  transition:
    transform var(--duration-slow),
    opacity var(--duration-base);
}

.collapsed .nav-group-chevron {
  display: none;
}

.nav-group-toggle.open .nav-group-chevron {
  transform: rotate(90deg);
}

.nav-group-items {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
  margin-top: var(--space-1);
}

.nav-subgroup-label {
  font-size: var(--fs-2xs);
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.04em;
  color: var(--text-muted);
  padding: var(--space-4) var(--space-5) var(--space-1) var(--space-9);
  white-space: nowrap;
  overflow: hidden;
}

.sidebar-footer {
  margin-top: auto;
  padding-top: var(--space-6);
  padding-bottom: calc(env(safe-area-inset-bottom, 0px) + var(--space-4));
  border-top: 1px solid var(--border);
  display: flex;
  flex-direction: column;
  gap: var(--space-5);
}

.user-info {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  padding: 0 var(--space-2);
  overflow: hidden;
}

.collapsed .user-info {
  display: none;
}

.user-name {
  font-size: var(--fs-sm);
  font-weight: 500;
  color: var(--text-primary);
  white-space: nowrap;
}

.user-role {
  font-size: var(--fs-2xs);
  font-weight: 600;
  text-transform: uppercase;
  padding: var(--space-1) var(--space-3);
  border-radius: var(--radius-sm);
}

.user-role.admin {
  color: var(--accent);
  background: var(--accent-subtle);
}

.user-role.user {
  color: var(--text-muted);
  background: var(--bg-hover);
}

.sidebar-actions {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  flex-wrap: wrap;
}

.signout-btn {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  font-size: var(--fs-xs);
  font-weight: 500;
  color: var(--text-muted);
  background: none;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  padding: var(--space-3) var(--space-4);
  cursor: pointer;
  transition:
    color var(--duration-base),
    border-color var(--duration-base);
  overflow: hidden;
  white-space: nowrap;
}

.collapsed .signout-btn {
  border: none;
  padding: var(--space-3);
  justify-content: center;
}

.collapsed .signout-btn .nav-label {
  display: none;
}

.signout-btn:hover {
  color: var(--danger);
  border-color: var(--danger);
}

.docs-btn {
  text-decoration: none;
}

.docs-btn:hover {
  color: var(--accent);
  border-color: var(--accent);
}

.content {
  flex: 1;
  padding: var(--space-9);
  min-width: 0;
  overflow-x: hidden;
}

/* Mobile header - hidden on desktop */
.mobile-header {
  display: none;
}

/* Backdrop - hidden by default */
.backdrop {
  display: none;
}

/* Responsive: mobile layout */
@media (max-width: 768px) {
  .mobile-header {
    display: flex;
    align-items: center;
    gap: var(--space-5);
    padding: var(--space-5) var(--space-6);
    background: var(--bg-sidebar);
    border-bottom: 1px solid var(--border);
    position: sticky;
    top: 0;
    z-index: 100;
  }

  .mobile-toggle {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 2.75rem;
    height: 2.75rem;
    border: none;
    background: transparent;
    color: var(--text-primary);
    border-radius: var(--radius-sm);
    cursor: pointer;
  }

  .mobile-toggle svg {
    width: 20px;
    height: 20px;
  }

  .mobile-toggle:hover {
    background: var(--bg-hover);
  }

  .mobile-brand {
    font-size: var(--fs-lg);
    font-weight: 700;
    color: var(--text-primary);
  }

  .shell {
    flex-direction: column;
  }

  .sidebar {
    position: fixed;
    top: 0;
    left: 0;
    bottom: 0;
    z-index: 200;
    width: 250px;
    transform: translateX(-100%);
    transition: transform var(--duration-slow) ease;
    will-change: transform;
  }

  .sidebar.mobile-open {
    transform: translateX(0);
  }

  /* Override desktop collapsed behavior on mobile */
  .collapsed .sidebar {
    width: 250px;
    padding: var(--space-7);
  }

  .collapsed .brand-text,
  .collapsed .nav-label,
  .collapsed .nav-group-chevron {
    display: inline;
    opacity: 1;
    width: auto;
    overflow: visible;
  }

  .collapsed .user-info {
    display: flex;
  }

  .collapsed .nav-link,
  .collapsed .nav-group-toggle {
    justify-content: flex-start;
    padding: var(--space-4) var(--space-5);
  }

  .collapsed .nav-link-nested {
    padding-left: var(--space-9);
  }

  .collapsed .signout-btn {
    border: 1px solid var(--border);
    padding: var(--space-3) var(--space-4);
    justify-content: flex-start;
  }

  .collapsed .signout-btn .nav-label {
    display: inline;
  }

  .collapsed .collapse-toggle {
    display: none;
  }

  .collapse-toggle {
    display: none;
  }

  .backdrop {
    display: block;
    position: fixed;
    inset: 0;
    z-index: 150;
    background: rgba(0, 0, 0, 0.4);
  }

  .content {
    padding: var(--space-7);
  }
}

.page-wrapper {
  position: relative;
}

.page-wrapper--dashboard {
  max-width: 1600px;
  margin: 0 auto;
}

.page-wrapper--constrained {
  max-width: 1100px;
  margin: 0 auto;
}

.page-enter-active,
.page-leave-active {
  transition: opacity var(--duration-base) ease;
}

.page-leave-active {
  position: absolute;
  inset: 0;
  pointer-events: none;
}

.page-enter-from,
.page-leave-to {
  opacity: 0;
}
</style>
