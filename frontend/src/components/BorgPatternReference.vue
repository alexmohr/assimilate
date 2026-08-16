<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
/**
 * Static cheat sheet for borg's exclude-pattern prefixes, shown inline next to
 * a pattern textarea. See docs/contributing/ui-design-audit.md (F-24).
 */
</script>

<template>
  <div class="ref-panel">
    <div class="ref-title">Borg Pattern Syntax</div>
    <div class="ref-section">
      <div class="ref-section-title">Shell Patterns (default)</div>
      <div class="ref-entry"><code>*.cache</code> <span>any file ending in .cache</span></div>
      <div class="ref-entry">
        <code>home/*/Downloads</code> <span>Downloads in any home dir</span>
      </div>
    </div>
    <div class="ref-section">
      <div class="ref-section-title">Path Prefix <code>pp:</code></div>
      <div class="ref-entry">
        <code>pp:__pycache__</code>
        <span>any path component named __pycache__</span>
      </div>
    </div>
    <div class="ref-section">
      <div class="ref-section-title">Regex <code>re:</code></div>
      <div class="ref-entry">
        <code>re:\.git/objects/</code> <span>regex match anywhere in path</span>
      </div>
    </div>
    <div class="ref-section">
      <div class="ref-section-title">Fnmatch <code>fm:</code></div>
      <div class="ref-entry">
        <code>fm:*.log</code> <span>fnmatch pattern (case-sensitive)</span>
      </div>
    </div>
  </div>
</template>

<style scoped>
.ref-panel {
  margin-top: 0.5rem;
  background: var(--bg-base);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  padding: 0.875rem;
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
}

.ref-title {
  font-size: var(--fs-xs);
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0.06em;
  color: var(--text-muted);
  padding-bottom: 0.5rem;
  border-bottom: 1px solid var(--border);
}

.ref-section {
  display: flex;
  flex-direction: column;
  gap: 0.35rem;
}

.ref-section-title {
  font-size: var(--fs-2xs);
  font-weight: 600;
  color: var(--text-muted);
  text-transform: uppercase;
  letter-spacing: 0.05em;
}

.ref-section-title code {
  font-family: var(--mono);
  color: var(--accent);
  text-transform: none;
  letter-spacing: 0;
  background: transparent;
  padding: 0;
}

.ref-entry {
  display: flex;
  align-items: baseline;
  gap: 0.5rem;
}

.ref-entry code {
  font-family: var(--mono);
  font-size: var(--fs-xs);
  color: var(--text-primary);
  background: var(--bg-card);
  padding: 0.1rem 0.35rem;
  border-radius: var(--radius-sm);
}

.ref-entry span {
  font-size: var(--fs-2xs);
  color: var(--text-muted);
}
</style>
