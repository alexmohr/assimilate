// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { createRouter, createWebHistory, type RouteRecordRaw } from 'vue-router'
import { useAuthStore } from '../stores/auth'

const routes: RouteRecordRaw[] = [
  {
    path: '/login',
    component: () => import('../views/LoginView.vue'),
    name: 'login',
    meta: { public: true },
  },
  {
    path: '/change-password',
    component: () => import('../views/ChangePasswordView.vue'),
    name: 'change-password',
    meta: { changePassword: true },
  },
  { path: '/', component: () => import('../views/DashboardView.vue'), name: 'dashboard' },
  { path: '/clients', component: () => import('../views/HostsView.vue'), name: 'clients' },
  {
    path: '/clients/:hostname',
    component: () => import('../views/HostDetailView.vue'),
    name: 'client-detail',
    props: true,
  },
  { path: '/repos', component: () => import('../views/ReposView.vue'), name: 'repos' },
  {
    path: '/repos/:id',
    component: () => import('../views/RepoDetailView.vue'),
    name: 'repo-detail',
    props: true,
  },
  { path: '/machines', redirect: '/clients' },
  { path: '/excludes', component: () => import('../views/ExcludesView.vue'), name: 'excludes' },
  { path: '/archives', redirect: '/repos' },
  { path: '/schedules', component: () => import('../views/SchedulesView.vue'), name: 'schedules' },
  {
    path: '/schedules/new',
    component: () => import('../views/ScheduleDetailView.vue'),
    name: 'schedule-create',
    props: { id: 'new' },
  },
  {
    path: '/schedules/:id',
    component: () => import('../views/ScheduleDetailView.vue'),
    name: 'schedule-detail',
    props: true,
  },
  { path: '/activity', component: () => import('../views/ActivityLogView.vue'), name: 'activity' },
  {
    path: '/users',
    component: () => import('../views/UsersView.vue'),
    name: 'users',
    meta: { requiresAdmin: true },
  },
  {
    path: '/system',
    component: () => import('../views/SystemView.vue'),
    name: 'system',
    meta: { requiresAdmin: true },
  },
  {
    path: '/admin/groups',
    component: () => import('../views/GroupsView.vue'),
    name: 'admin-groups',
    meta: { requiresAdmin: true },
  },
  {
    path: '/admin/roles',
    component: () => import('../views/RolesView.vue'),
    name: 'admin-roles',
    meta: { requiresAdmin: true },
  },
  {
    path: '/tunnels',
    name: 'tunnels',
    component: () => import('../views/TunnelsView.vue'),
    meta: { requiresAdmin: true },
  },
  { path: '/tokens', component: () => import('../views/TokensView.vue'), name: 'tokens' },
  { path: '/profile', component: () => import('../views/ProfileView.vue'), name: 'profile' },
  { path: '/appearance', redirect: '/profile' },
  {
    path: '/error',
    component: () => import('../views/ErrorView.vue'),
    name: 'error',
    meta: { public: true },
  },
  {
    path: '/:pathMatch(.*)*',
    component: () => import('../views/NotFoundView.vue'),
    name: 'not-found',
    meta: { public: true },
  },
]

export const router = createRouter({
  history: createWebHistory(),
  routes,
})

router.beforeEach(async (to) => {
  if (to.meta.public) {
    return true
  }

  const authStore = useAuthStore()

  if (!authStore.user) {
    await authStore.fetchMe()
  }

  if (!authStore.user) {
    return { name: 'login' }
  }

  if (authStore.user.must_change_password && !to.meta.changePassword) {
    return { name: 'change-password' }
  }

  if (to.meta.requiresAdmin && authStore.user.role !== 'admin') {
    return { name: 'dashboard' }
  }

  return true
})
