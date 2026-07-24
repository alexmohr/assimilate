<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useAuthStore } from '../stores/auth'
import { useAsyncAction } from '../composables/useAsyncAction'
const authStore = useAuthStore()
const route = useRoute()
const router = useRouter()

const username = ref('')
const password = ref('')
const rememberMe = ref(false)
const totpCode = ref('')
const { loading: submitting, error, run } = useAsyncAction('Login failed')

watch(
  () => authStore.user,
  (newUser) => {
    if (newUser) {
      const next =
        typeof route.query.next === 'string' && route.query.next.startsWith('/')
          ? route.query.next
          : '/'
      if (newUser.must_change_password) {
        router.push({ path: '/change-password', query: { next } })
      } else {
        router.push(next)
      }
    }
  },
)

async function handleSubmit(): Promise<void> {
  if (authStore.totpRequired) {
    await run(async () => {
      await authStore.verifyTotp(totpCode.value)
    })
    return
  }

  await run(async () => {
    await authStore.login(username.value, password.value, rememberMe.value)
    // If TOTP is required, we don't redirect - stay to show TOTP input
    if (!authStore.totpRequired) {
      const next =
        typeof route.query.next === 'string' && route.query.next.startsWith('/')
          ? route.query.next
          : '/'
      if (authStore.user?.must_change_password) {
        router.push({ path: '/change-password', query: { next } })
      } else {
        router.push(next)
      }
    }
  })
}

function handleBackToLogin(): void {
  authStore.resetTotpState()
  totpCode.value = ''
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
        v-if="!authStore.totpRequired"
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
          <label for="remember-me">Remember me</label>
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

      <form
        v-else
        class="login-form"
        @submit.prevent="handleSubmit"
      >
        <div class="totp-info">
          <p class="totp-info-text">Two-factor authentication is required for this account.</p>
          <p class="totp-info-subtext">Enter the code from your authenticator app.</p>
        </div>

        <div class="form-group">
          <label for="totp-code">Authenticator Code</label>
          <input
            id="totp-code"
            v-model="totpCode"
            type="text"
            inputmode="numeric"
            autocomplete="one-time-code"
            placeholder="000000"
            required
            maxlength="6"
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
          <span v-if="submitting">Verifying...</span>
          <span v-else>Verify</span>
        </button>

        <button
          type="button"
          class="login-btn login-btn-ghost"
          :disabled="submitting"
          @click="handleBackToLogin"
        >
          Back to login
        </button>
      </form>
    </div>
  </div>
</template>

<style>
@import url('../assets/auth.css');
</style>

<style scoped>
.login-icon {
  border-radius: 8px;
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

.login-btn-ghost {
  background: transparent;
  color: var(--text-secondary);
  border: 1px solid var(--border);
}

.login-btn-ghost:hover:not(:disabled) {
  background: var(--bg-hover);
}

.totp-info {
  text-align: center;
  margin-bottom: 0.5rem;
}

.totp-info-text {
  font-size: 0.9rem;
  font-weight: 600;
  color: var(--text-primary);
  margin: 0 0 0.25rem;
}

.totp-info-subtext {
  font-size: 0.8rem;
  color: var(--text-muted);
  margin: 0;
}
</style>
