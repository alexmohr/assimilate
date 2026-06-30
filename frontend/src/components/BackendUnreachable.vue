<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { WifiOff } from '@lucide/vue'
import { useBackendStatus } from '../composables/useBackendStatus'

const { backendUnreachable, retryCountdown, checking, retryNow } = useBackendStatus()
</script>

<template>
  <Transition name="fade">
    <div
      v-if="backendUnreachable"
      class="backend-overlay"
    >
      <div class="backend-overlay-card">
        <WifiOff
          :size="48"
          class="backend-overlay-icon"
        />
        <h2 class="backend-overlay-title">Trying to reach backend</h2>
        <p class="backend-overlay-subtitle">
          The server is not responding. Retrying in
          <strong>{{ retryCountdown }}s</strong>
        </p>
        <button
          class="backend-overlay-btn"
          :disabled="checking"
          @click="retryNow()"
        >
          {{ checking ? 'Checking\u2026' : 'Retry now' }}
        </button>
      </div>
    </div>
  </Transition>
</template>

<style scoped>
.backend-overlay {
  position: fixed;
  inset: 0;
  z-index: 9999;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(0, 0, 0, 0.6);
  backdrop-filter: blur(4px);
}

.backend-overlay-card {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 1rem;
  padding: 2.5rem 3rem;
  background: var(--bg-card);
  border-radius: var(--radius-lg, 12px);
  border: 1px solid var(--border);
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.2);
  text-align: center;
  max-width: 360px;
}

.backend-overlay-icon {
  color: var(--warning, #f59e0b);
}

.backend-overlay-title {
  margin: 0;
  font-size: 1.25rem;
  font-weight: 600;
  color: var(--text-primary);
}

.backend-overlay-subtitle {
  margin: 0;
  font-size: 0.875rem;
  color: var(--text-secondary);
}

.backend-overlay-btn {
  margin-top: 0.5rem;
  padding: 0.5rem 1.5rem;
  font-size: 0.875rem;
  font-weight: 500;
  border: none;
  border-radius: var(--radius-sm, 6px);
  background: var(--accent, #3b82f6);
  color: #fff;
  cursor: pointer;
  transition:
    background 0.15s,
    opacity 0.15s;
}

.backend-overlay-btn:hover:not(:disabled) {
  opacity: 0.9;
}

.backend-overlay-btn:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.fade-enter-active,
.fade-leave-active {
  transition: opacity 0.2s ease;
}

.fade-enter-from,
.fade-leave-to {
  opacity: 0;
}
</style>
