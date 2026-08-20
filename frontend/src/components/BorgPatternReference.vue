<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
/**
 * Static cheat sheet for borg's exclude-pattern prefixes. `inline` sits under a
 * pattern textarea, `sidebar` is the sticky column on the global excludes page.
 * Both used to carry their own copy of the markup, and the two copies had
 * already drifted apart in which examples they listed.
 */

interface PatternEntry {
  pattern: string
  meaning: string
}

interface PatternSection {
  title: string
  /** The prefix the section documents, rendered as code beside the title. */
  prefix?: string
  entries: PatternEntry[]
}

withDefaults(defineProps<{ variant?: 'inline' | 'sidebar' }>(), { variant: 'inline' })

const SECTIONS: readonly PatternSection[] = [
  {
    title: 'Shell Patterns (default)',
    entries: [
      { pattern: '*.cache', meaning: 'any file ending in .cache' },
      { pattern: 'home/*/Downloads', meaning: 'Downloads in any home dir' },
      { pattern: '*.{jpg,png}', meaning: 'multiple extensions' },
    ],
  },
  {
    title: 'Path Prefix',
    prefix: 'pp:',
    entries: [
      { pattern: 'pp:__pycache__', meaning: 'any path component named __pycache__' },
      { pattern: 'pp:/proc', meaning: 'exact path prefix /proc' },
    ],
  },
  {
    title: 'Regex',
    prefix: 're:',
    entries: [
      { pattern: 're:\\.git/objects/', meaning: 'regex match anywhere in path' },
      { pattern: 're:/tmp/[^/]+\\.sock$', meaning: 'socket files in /tmp' },
    ],
  },
  {
    title: 'Fnmatch',
    prefix: 'fm:',
    entries: [{ pattern: 'fm:*.log', meaning: 'fnmatch pattern (case-sensitive)' }],
  },
]
</script>

<template>
  <div
    class="ref-panel"
    :class="`ref-panel--${variant}`"
  >
    <div class="ref-title">Borg Pattern Syntax</div>
    <div
      v-for="section in SECTIONS"
      :key="section.title"
      class="ref-section"
    >
      <div class="group-label ref-section-title">
        {{ section.title }}
        <code v-if="section.prefix">{{ section.prefix }}</code>
      </div>
      <div
        v-for="entry in section.entries"
        :key="entry.pattern"
        class="ref-entry"
      >
        <code>{{ entry.pattern }}</code>
        <span>{{ entry.meaning }}</span>
      </div>
    </div>
    <div
      v-if="$slots.note"
      class="ref-note"
    >
      <slot name="note" />
    </div>
  </div>
</template>

<style scoped>
.ref-panel {
  background: var(--bg-base);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  padding: var(--space-5);
  display: flex;
  flex-direction: column;
  gap: var(--space-5);
}

.ref-panel--inline {
  margin-top: var(--space-4);
}

/* The excludes page shows it as a sticky column beside the editor. */
.ref-panel--sidebar {
  width: 280px;
  flex-shrink: 0;
  background: var(--bg-card);
  border-radius: var(--radius);
  padding: var(--space-7);
  gap: var(--space-6);
  position: sticky;
  top: 1rem;
}

.ref-title {
  font-size: var(--fs-xs);
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0.06em;
  color: var(--text-muted);
  padding-bottom: var(--space-4);
  border-bottom: 1px solid var(--border);
}

.ref-panel--sidebar .ref-title {
  font-size: var(--fs-base);
  padding-bottom: var(--space-5);
}

.ref-section {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}

.ref-panel--sidebar .ref-section-title {
  font-size: var(--fs-xs);
  margin-bottom: var(--space-2);
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
  gap: var(--space-4);
}

/* The sidebar is too narrow for a code/meaning pair on one line. */
.ref-panel--sidebar .ref-entry {
  flex-direction: column;
  align-items: stretch;
  gap: var(--space-1);
}

.ref-entry code {
  font-family: var(--mono);
  font-size: var(--fs-xs);
  color: var(--text-primary);
  background: var(--bg-card);
  padding: var(--space-1) var(--space-3);
  border-radius: var(--radius-sm);
}

.ref-panel--sidebar .ref-entry code {
  font-size: var(--fs-sm);
  display: inline-block;
  background: var(--bg-base);
  padding: var(--space-1) var(--space-3);
}

.ref-entry span {
  font-size: var(--fs-2xs);
  color: var(--text-muted);
}

.ref-note {
  font-size: var(--fs-xs);
  color: var(--text-muted);
  line-height: 1.5;
  padding-top: var(--space-4);
  border-top: 1px solid var(--border);
}

.ref-panel--sidebar .ref-entry span {
  font-size: var(--fs-xs);
  padding-left: var(--space-2);
}
</style>
