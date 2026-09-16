<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
/**
 * One setting inside a `.settings-tab` pane: what it is on the left - its
 * name, its `HelpHint` and, where a unit or a default needs saying, a
 * `.field-hint` - and its control on the right, in a track of its own.
 *
 * A component rather than a documented block of markup because the block is
 * twelve lines of wrappers around a two-line control, repeated some seventy
 * times across the three detail pages. Written out by hand it drifted (a
 * label beside its control on one pane and above it on the next), and two
 * panes that happened to write the same toggle row tripped the duplicate-code
 * check. Neither is possible through here.
 *
 * The `{{ ' ' }}` before the `HelpHint` is deliberate: the gap between a label
 * and its `?` used to be incidental template whitespace at every call site,
 * and Vue's condense mode drops that between two elements. Here it is a real
 * text node, so the gap is not something each caller has to remember to type.
 */
import HelpHint from './HelpHint.vue'

withDefaults(
  defineProps<{
    /** The setting's name, on the left of the row. */
    title?: string
    /** `for` target, when the title labels one specific control. */
    labelFor?: string
    /**
     * A unit, a default, a sentinel value or a live reading - never a
     * what-it-does sentence, which belongs behind `help`. The rule is whether
     * the sentence would still be true with the field left empty.
     */
    hint?: string
    /**
     * `HelpHint`'s short aria label ("running once after an outage"); the
     * sentence it discloses is the `help` slot.
     */
    help?: string
    /**
     * A control that is an editor - a textarea, a pattern list, a cron
     * builder, a block per agent, a run of fields - keeps the label column
     * and takes the full width underneath instead of a 220px track.
     */
    stack?: boolean
  }>(),
  { title: undefined, labelFor: undefined, hint: undefined, help: undefined, stack: false },
)
</script>

<template>
  <div
    class="pane-row"
    :class="{ 'pane-row--stack': stack }"
  >
    <div class="field-body">
      <p
        class="field-title"
        :class="{ 'field-label-row': $slots.titleAside }"
      >
        <span>
          <label
            v-if="labelFor"
            :for="labelFor"
            >{{ title }}</label
          >
          <template v-else>{{ title }}</template>
          <slot name="titleExtra" />
          <template v-if="help">
            {{ ' ' }}
            <HelpHint :label="help">
              <slot name="help" />
            </HelpHint>
          </template>
        </span>
        <slot name="titleAside" />
      </p>
      <span
        v-if="hint || $slots.hint"
        class="field-hint"
      >
        <slot name="hint">{{ hint }}</slot>
      </span>
    </div>
    <div class="pane-row-control">
      <slot />
    </div>
  </div>
</template>
