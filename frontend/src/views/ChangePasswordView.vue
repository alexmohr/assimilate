<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useAuthStore } from '../stores/auth'
import { validatePassword } from '../utils/validation'
import { extractError } from '../utils/error'
const authStore = useAuthStore()
const route = useRoute()
const router = useRouter()

const newPassword = ref('')
const confirmPassword = ref('')
const error = ref('')
const submitting = ref(false)

async function handleSubmit(): Promise<void> {
  error.value = ''

  const validationError = validatePassword(newPassword.value, confirmPassword.value)
  if (validationError) {
    error.value = validationError
    return
  }

  submitting.value = true
  try {
    await authStore.changePassword(newPassword.value)
    const next = typeof route.query.next === 'string' && route.query.next.startsWith('/')
      ? route.query.next
      : '/'
    router.push(next)
  } catch (e: unknown) {
    error.value = extractError(e, 'Failed to change password')
  } finally {
    submitting.value = false
  }
}
</script>

<template>
  <div class="login-page">
    <div class="login-card">
      <div class="login-header">
        <svg
          class="login-icon"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
          width="32"
          height="32"
        >
          <rect
            x="3"
            y="11"
            width="18"
            height="11"
            rx="2"
            ry="2"
          />
          <path d="M7 11V7a5 5 0 0 1 10 0v4" />
        </svg>
        <h1 class="login-title">Change Password</h1>
        <p class="login-subtitle">You must change your password before continuing.</p>
      </div>

      <form
        class="login-form"
        @submit.prevent="handleSubmit"
      >
        <div class="form-group">
          <label for="new-password">New Password</label>
          <input
            id="new-password"
            v-model="newPassword"
            type="password"
            autocomplete="new-password"
            required
            :disabled="submitting"
          />
        </div>

        <div class="form-group">
          <label for="confirm-password">Confirm Password</label>
          <input
            id="confirm-password"
            v-model="confirmPassword"
            type="password"
            autocomplete="new-password"
            required
            :disabled="submitting"
          />
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
          <span v-if="submitting">Saving...</span>
          <span v-else>Set New Password</span>
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
  color: var(--accent);
}

.login-title {
  font-size: 1.25rem;
  font-weight: 700;
  color: var(--text-primary);
  margin: 0;
}

.login-subtitle {
  font-size: 0.8125rem;
  color: var(--text-secondary);
  text-align: center;
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
