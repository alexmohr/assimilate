<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { useRoute, useRouter } from 'vue-router'

const route = useRoute()
const router = useRouter()

const statusCode = route.query.code ?? '500'
const message = route.query.message ?? 'Something went wrong. Please try again later.'

function goHome(): void {
  router.push('/')
}
</script>

<template>
  <div class="error-page">
    <div class="error-card">
      <div class="error-code">
        {{ statusCode }}
      </div>
      <h1 class="error-title">Error</h1>
      <p class="error-message">
        {{ message }}
      </p>
      <button
        class="error-btn"
        @click="goHome"
      >
        Back to Dashboard
      </button>
    </div>
  </div>
</template>

<style scoped>
.error-page {
  display: flex;
  align-items: center;
  justify-content: center;
  min-height: 100vh;
  background: var(--bg-base);
  padding: 1rem;
}

.error-card {
  text-align: center;
  max-width: 420px;
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 3rem 2rem;
  box-shadow: var(--shadow-lg);
}

.error-code {
  font-size: 4rem;
  font-weight: 800;
  color: var(--danger);
  line-height: 1;
  margin-bottom: 0.5rem;
}

.error-title {
  font-size: 1.25rem;
  font-weight: 700;
  color: var(--text-primary);
  margin: 0 0 0.5rem;
}

.error-message {
  font-size: 0.875rem;
  color: var(--text-secondary);
  margin: 0 0 1.5rem;
  line-height: 1.5;
}

.error-btn {
  padding: 0.625rem 1.25rem;
  background: var(--accent);
  color: #fff;
  border: none;
  border-radius: var(--radius-sm);
  font-size: 0.875rem;
  font-weight: 600;
  cursor: pointer;
  transition: background 0.15s;
}

.error-btn:hover {
  background: var(--accent-hover);
}
</style>
