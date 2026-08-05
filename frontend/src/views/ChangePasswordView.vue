<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useAuthStore } from '../stores/auth'
import { validatePassword } from '../utils/validation'
import { useAsyncAction } from '../composables/useAsyncAction'
const authStore = useAuthStore()
const route = useRoute()
const router = useRouter()

const newPassword = ref('')
const confirmPassword = ref('')
const { loading: submitting, error, run } = useAsyncAction('Failed to change password')

async function handleSubmit(): Promise<void> {
  const validationError = validatePassword(newPassword.value, confirmPassword.value)
  if (validationError) {
    error.value = validationError
    return
  }

  await run(async () => {
    await authStore.changePassword(newPassword.value)
    const next =
      typeof route.query.next === 'string' && route.query.next.startsWith('/')
        ? route.query.next
        : '/'
    router.push(next)
  })
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

<style>
@import url('../assets/auth.css');
</style>
