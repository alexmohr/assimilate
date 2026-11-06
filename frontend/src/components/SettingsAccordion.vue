<script setup lang="ts" generic="T extends string">
import { shallowRef, watch } from 'vue'
import { ChevronRight } from '@lucide/vue'
import type { SettingsSections } from './SettingsRail.vue'

/**
 * Collapsible sections inside one bordered box, the same box the Overview
 * tab's repository rows sit in. Replaces the rail on a settings tab: a
 * second row of tabs under the page's own tab bar wrapped on a phone and
 * read as two competing navigations. Several sections may be open at once.
 */
const props = defineProps<{
  sections: SettingsSections<T>
  /** The section named in the URL; it starts open. */
  section: T
  /** Names the group for assistive tech, e.g. "Agent settings sections". */
  label: string
}>()

const emit = defineEmits<{ 'update:section': [value: T] }>()

function initialOpen(): Set<T> {
  const requested = props.sections.some((s) => s.id === props.section)
    ? props.section
    : props.sections[0].id
  return new Set([requested])
}

const open = shallowRef<ReadonlySet<T>>(initialOpen())

watch(
  () => props.section,
  (s) => {
    if (props.sections.some((x) => x.id === s)) open.value = new Set([...open.value, s])
  },
)

function toggle(id: T): void {
  const next = new Set(open.value)
  if (next.has(id)) {
    next.delete(id)
  } else {
    next.add(id)
    emit('update:section', id)
  }
  open.value = next
}

function setAll(expand: boolean): void {
  open.value = expand ? new Set(props.sections.map((s) => s.id)) : new Set()
}
</script>

<template>
  <div
    class="accordion"
    role="group"
    :aria-label="label"
  >
    <div
      v-if="sections.length > 1"
      class="accordion-tools"
    >
      <button
        type="button"
        class="btn btn-sm btn-ghost"
        @click="setAll(open.size < sections.length)"
      >
        {{ open.size < sections.length ? 'Expand all' : 'Collapse all' }}
      </button>
    </div>
    <div class="rows">
      <section
        v-for="s in sections"
        :key="s.id"
        class="accordion-item"
      >
        <button
          :id="`acc-head-${s.id}`"
          type="button"
          class="accordion-head"
          :aria-expanded="open.has(s.id)"
          :aria-controls="`acc-body-${s.id}`"
          @click="toggle(s.id)"
        >
          <ChevronRight
            :size="16"
            class="accordion-chevron"
            :class="{ 'accordion-chevron--open': open.has(s.id) }"
          />
          <span class="accordion-title">{{ s.label }}</span>
        </button>
        <div
          v-if="open.has(s.id)"
          :id="`acc-body-${s.id}`"
          class="accordion-body settings-pane"
          role="region"
          :aria-labelledby="`acc-head-${s.id}`"
        >
          <slot :section="s.id" />
        </div>
      </section>
    </div>
  </div>
</template>

<style scoped>
.accordion-tools {
  display: flex;
  justify-content: flex-end;
  margin-bottom: var(--space-3);
}

.accordion-item + .accordion-item {
  border-top: 1px solid var(--border-subtle);
}

.accordion-head {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  width: 100%;
  padding: var(--space-5) var(--space-6);
  border: none;
  background: none;
  font: inherit;
  font-weight: 600;
  color: var(--text-primary);
  text-align: left;
  cursor: pointer;
}

.accordion-head:hover {
  background: var(--bg-hover);
}

.accordion-chevron {
  flex: none;
  color: var(--text-muted);
  transition: transform var(--duration-base);
}

.accordion-chevron--open {
  transform: rotate(90deg);
}

.accordion-title {
  flex: 1;
  min-width: 0;
}

.accordion-body {
  border-top: 1px solid var(--border-subtle);
  padding: var(--space-6);
}

/* The box already frames the section; the nest's left rule would be a
   third vertical line beside the box edge and the indent. */
.accordion-body :deep(.pane-nest) {
  border-left: none;
  padding-left: 0;
  margin-left: 0;
}

@media (prefers-reduced-motion: reduce) {
  .accordion-chevron {
    transition: none;
  }
}
</style>
