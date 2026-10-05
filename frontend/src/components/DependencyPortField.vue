<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, ref } from 'vue'
import BaseSegmented from './BaseSegmented.vue'
import {
  PORT_PRESET_OPTIONS,
  presetForPort,
  presetPort,
  type PortPreset,
} from '../utils/dependencyHost'

/**
 * Which port says a dependency is up: a well-known one picked by name, or any
 * other typed in. Shared by the New dialog and the Connection section so the
 * two cannot drift into offering different presets.
 *
 * "Other port" sticks once chosen, even if the number typed happens to be a
 * preset's: re-deriving the choice from the port on every keystroke would hide
 * the input while someone is still typing "4450".
 */
const props = defineProps<{
  /** `id` of the number input, for a label's `for`. */
  inputId: string
}>()

const port = defineModel<number>({ required: true })

const otherChosen = ref(presetForPort(port.value) === 'other')

const preset = computed<PortPreset>({
  get: () => (otherChosen.value ? 'other' : presetForPort(port.value)),
  set: (next: PortPreset) => {
    const known = presetPort(next)
    otherChosen.value = known === null
    if (known !== null) port.value = known
  },
})
</script>

<template>
  <div class="dependency-port">
    <BaseSegmented
      v-model="preset"
      :options="PORT_PRESET_OPTIONS"
      label="Check"
    />
    <input
      v-if="preset === 'other'"
      :id="props.inputId"
      v-model.number="port"
      class="input field-narrow"
      type="number"
      min="1"
      max="65535"
      aria-label="Port"
    />
  </div>
</template>

<style scoped>
.dependency-port {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-4);
}
</style>
