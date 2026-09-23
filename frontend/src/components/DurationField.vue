<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, ref } from 'vue'
import { type DurationUnit, inUnit, naturalUnit, toMinutes } from '../utils/duration'

/**
 * A minute-valued setting, edited in whichever unit suits it.
 *
 * The three catch-up fields all store minutes and all want a different unit to
 * read in - a re-check interval in minutes, a collision floor in hours, a
 * give-up window in days - and each one was going to grow its own copy of the
 * same number-plus-select markup and the same rounding. One component instead,
 * with the units it offers as a prop.
 *
 * The displayed unit starts as whichever the stored value reads naturally in,
 * and stops moving once the user picks one: re-deriving it on every keystroke
 * would flip "1" from hours to minutes the moment a "20" was typed after it.
 */
const props = withDefaults(
  defineProps<{
    /** Units this field offers, finest first. */
    units: readonly DurationUnit[]
    /** `id` of the number input, for a `PaneRow`'s `label-for`. */
    inputId?: string
    /** Accessible name for the unit selector. */
    unitLabel: string
    /**
     * Whether an empty field is meaningful. When true, clearing the input
     * stores 0 and a stored 0 renders as empty - the "leave empty to..."
     * sentinel. When false, the field holds at least 1.
     */
    clearable?: boolean
    /** Greys the whole field out, for a setting that currently applies to nothing. */
    disabled?: boolean
  }>(),
  { inputId: undefined, clearable: false, disabled: false },
)

const minutes = defineModel<number>({ required: true })

const unitChoice = ref<DurationUnit | null>(null)

const unit = computed<DurationUnit>({
  get: () => unitChoice.value ?? naturalUnit(minutes.value, props.units),
  set: (next: DurationUnit) => {
    unitChoice.value = next
  },
})

/**
 * Empty rather than 0 for a clearable field: the hint beside it says "leave
 * empty", so showing a 0 there would be showing a different sentinel than the
 * one documented.
 */
const value = computed<number | null>({
  get: () => {
    if (props.clearable && minutes.value === 0) return null
    return inUnit(minutes.value, unit.value)
  },
  set: (next: number | null) => {
    if (next === null || !Number.isFinite(next)) {
      minutes.value = props.clearable ? 0 : 1
      return
    }
    const floor = props.clearable ? 0 : 1
    minutes.value = Math.max(floor, toMinutes(next, unit.value))
  },
})
</script>

<template>
  <div class="field-row">
    <input
      :id="inputId"
      v-model.number="value"
      type="number"
      :min="clearable ? 0 : 1"
      :disabled="disabled"
      class="input field-narrow"
    />
    <select
      v-model="unit"
      class="input select-input select-input--sm"
      :aria-label="unitLabel"
      :disabled="disabled"
    >
      <option
        v-for="option in units"
        :key="option"
        :value="option"
      >
        {{ option }}
      </option>
    </select>
    <slot />
  </div>
</template>
