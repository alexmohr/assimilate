<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts" generic="T extends string">
import { computed } from 'vue'

/**
 * The rail-and-pane shell every settings tab uses: a narrow list of sections
 * down the side, the current one's content beside it.
 *
 * Agents, schedules and repositories each grew their own copy of the nav
 * markup, the fallback rule and the mobile collapse. The rule is the part
 * worth having once: `?section=` comes from the URL, so a section this viewer
 * has no button for must not render behind their back just because they typed
 * its name - it falls back to the first section they do have.
 *
 * The rail names the current section, so the pane it wraps does not repeat it
 * - see `.pane-head` / `.pane-lede` in `skills/ui-design/SKILL.md`.
 */
export interface SettingsSectionOption<V extends string> {
  id: V
  label: string
  /** Tones the rail item red, for a section that destroys something. */
  danger?: boolean
}

/**
 * A rail's sections, of which there is always at least one - a rail with no
 * sections has no first section to fall back to, and a settings tab showing
 * nothing at all is not a state any caller wants. The tuple says so to the
 * compiler, so callers whose list shrinks by role cannot shrink it to nothing
 * and no unreachable empty-state branch has to be written or tested here.
 */
export type SettingsSections<V extends string> = readonly [
  SettingsSectionOption<V>,
  ...SettingsSectionOption<V>[],
]

const props = defineProps<{
  sections: SettingsSections<T>
  /** The requested section, which may be one this viewer cannot open. */
  section: T
  /** Names the rail for assistive tech, e.g. "Agent settings sections". */
  label: string
  /**
   * Spreads the sections into equal cells when the rail collapses, rather
   * than letting them wrap. For a rail with a small, fixed set of sections
   * that fits one row at any width.
   */
  even?: boolean
}>()

const emit = defineEmits<{ 'update:section': [value: T] }>()

/** What the pane renders and what the rail marks current - deliberately not
    the `section` prop. */
const current = computed<T>(() =>
  props.sections.some((s) => s.id === props.section) ? props.section : props.sections[0].id,
)
</script>

<template>
  <div class="settings-tab">
    <nav
      class="settings-nav"
      :class="{ 'settings-nav--even': even }"
      :aria-label="label"
    >
      <button
        v-for="s in sections"
        :key="s.id"
        type="button"
        class="settings-nav-item"
        :class="{ 'settings-nav-item--danger': s.danger }"
        :aria-current="s.id === current"
        @click="emit('update:section', s.id)"
      >
        {{ s.label }}
      </button>
    </nav>

    <div class="settings-pane">
      <slot :section="current" />
    </div>
  </div>
</template>
