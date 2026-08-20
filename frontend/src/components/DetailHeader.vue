<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
/**
 * The identity block at the top of a detail page, shown above the tab strip
 * and so present on every tab: what this thing is called, what state it is in,
 * what is known about it, and what can be done to it.
 *
 * Agents and schedules each had their own copy of this - the same markup and
 * the same eleven CSS rules under an `.agent-` and a `.schedule-` prefix - and
 * repositories had none at all, so a repository's name appeared only as the
 * last crumb of its breadcrumb and the page offered no action anywhere.
 */
defineProps<{
  /** The entity's name, as the page's `h1`. */
  name: string
  /** Renders the name in the monospace face, for machine identifiers. */
  mono?: boolean
  /**
   * Renders the meta line in the monospace face. Same distinction as `mono`:
   * versions, revisions and build stamps line up in it; counts and sizes read
   * better in the proportional face.
   */
  monoMeta?: boolean
  /** The line under the name: a display name, a type and cadence, a path. */
  subtitle?: string | null
}>()
</script>

<template>
  <header class="detail-header">
    <div class="detail-identity">
      <div class="detail-title-row">
        <h1
          class="detail-name"
          :class="{ 'detail-name--mono': mono }"
        >
          {{ name }}
        </h1>
        <slot name="badges" />
      </div>
      <p
        v-if="subtitle"
        class="detail-subtitle"
      >
        {{ subtitle }}
      </p>
      <div
        v-if="$slots.meta"
        class="detail-meta"
        :class="{ 'detail-meta--mono': monoMeta }"
      >
        <slot name="meta" />
      </div>
    </div>

    <div class="detail-actions">
      <slot name="actions" />
    </div>

    <slot name="footer" />
  </header>
</template>
