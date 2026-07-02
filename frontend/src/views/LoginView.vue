<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useAuthStore } from '../stores/auth'
import { useAsyncAction } from '../composables/useAsyncAction'
const authStore = useAuthStore()
const route = useRoute()
const router = useRouter()

const username = ref('')
const password = ref('')
const rememberMe = ref(false)
const { loading: submitting, error, run } = useAsyncAction('Login failed')

async function handleSubmit(): Promise<void> {
  await run(async () => {
    await authStore.login(username.value, password.value, rememberMe.value)
    const next =
      typeof route.query.next === 'string' && route.query.next.startsWith('/')
        ? route.query.next
        : '/'
    if (authStore.user?.must_change_password) {
      router.push({ path: '/change-password', query: { next } })
    } else {
      router.push(next)
    }
  })
}
</script>

<template>
  <div class="login-page">
    <div class="login-card">
      <div class="login-header">
        <img
          class="login-icon"
          src="/icon.png"
          alt="Assimilate"
          width="128"
          height="128"
        />
        <h1 class="login-title">Assimilate</h1>
      </div>

      <form
        class="login-form"
        @submit.prevent="handleSubmit"
      >
        <div class="form-group">
          <label for="username">Username</label>
          <input
            id="username"
            v-model="username"
            type="text"
            autocomplete="username"
            required
            :disabled="submitting"
          />
        </div>

        <div class="form-group">
          <label for="password">Password</label>
          <input
            id="password"
            v-model="password"
            type="password"
            autocomplete="current-password"
            required
            :disabled="submitting"
          />
        </div>

        <div class="remember-me">
          <input
            id="remember-me"
            v-model="rememberMe"
            type="checkbox"
            :disabled="submitting"
          />
          <label for="remember-me">Remember me for 30 days</label>
        </div>

        <div
          v-if="error"
          class="login-error"
        >
          {{ error }}
        </div>

        <button
          type="submit"
          class="login-btn"
          :disabled="submitting"
        >
          <span v-if="submitting">Signing in...</span>
          <span v-else>Sign in</span>
        </button>
      </form>
    </div>
  </div>
</template>

<style scoped>
.login-page {
  display: flex;
  align-items: center;
  justify-content: center;
  min-height: 100vh;
  background: var(--bg-base);
  padding: 1rem;
}

.login-card {
  width: 100%;
  max-width: 380px;
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 2rem;
  box-shadow: var(--shadow-lg);
}

.login-header {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 0.5rem;
  margin-bottom: 1.5rem;
}

.login-icon {
  border-radius: 8px;
}

.login-title {
  font-size: 1.25rem;
  font-weight: 700;
  color: var(--text-primary);
  margin: 0;
}

.login-form {
  display: flex;
  flex-direction: column;
  gap: 1rem;
}

.form-group {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
}

.form-group label {
  font-size: 0.8125rem;
  font-weight: 500;
  color: var(--text-secondary);
}

.form-group input {
  padding: 0.625rem 0.75rem;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg-input);
  color: var(--text-primary);
  font-size: 0.875rem;
  outline: none;
  transition: border-color 0.15s;
}

.form-group input:focus {
  border-color: var(--accent);
}

.remember-me {
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

.remember-me input[type='checkbox'] {
  width: 1rem;
  height: 1rem;
  accent-color: var(--accent);
  cursor: pointer;
}

.remember-me label {
  font-size: 0.8125rem;
  color: var(--text-secondary);
  cursor: pointer;
  user-select: none;
}

.login-error {
  font-size: 0.8125rem;
  color: var(--danger);
  padding: 0.5rem 0.75rem;
  background: var(--danger-subtle);
  border-radius: var(--radius-sm);
}

.login-btn {
  padding: 0.625rem 1rem;
  background: var(--accent);
  color: #fff;
  border: none;
  border-radius: var(--radius-sm);
  font-size: 0.875rem;
  font-weight: 600;
  cursor: pointer;
  transition: background 0.15s;
}

.login-btn:hover:not(:disabled) {
  background: var(--accent-hover);
}

.login-btn:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}
</style>
