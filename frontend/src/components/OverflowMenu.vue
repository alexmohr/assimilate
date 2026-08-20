<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref } from 'vue'
import { MoreHorizontal } from '@lucide/vue'
import { useOverflowMenu } from '../composables/useOverflowMenu'

/**
 * The "..." button on a detail header and the menu it opens.
 *
 * Every header grades its actions the same way: one accented slot for the
 * thing that is actionable right now, everything else - including navigation -
 * in here. The two headers that existed each carried their own copy of this
 * markup and of eleven identical CSS rules, under two different class
 * prefixes.
 *
 * The default slot receives `run`, which performs an action and closes the
 * menu, so no item has to remember to close it.
 */
defineProps<{
  /** Names the menu for assistive technology, e.g. "More agent actions". */
  label: string
}>()

const root = ref<HTMLElement | null>(null)
const { menuOpen, runAndClose } = useOverflowMenu(root)
</script>

<template>
  <div
    ref="root"
    class="overflow-wrap"
  >
    <button
      class="btn btn-sm btn-ghost overflow-toggle"
      type="button"
      aria-haspopup="menu"
      :aria-expanded="menuOpen"
      :aria-label="label"
      @click="menuOpen = !menuOpen"
    >
      <MoreHorizontal :size="14" />
    </button>

    <div
      v-if="menuOpen"
      class="overflow-menu"
      role="menu"
    >
      <slot :run="runAndClose" />
    </div>
  </div>
</template>

<style scoped>
/* The menu positions against this wrapper rather than against the actions row,
   so a header can put other buttons beside it without moving the menu. */
.overflow-wrap {
  position: relative;
  display: flex;
}
</style>
