<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import EditableSection from './EditableSection.vue'
import PaneRow from './PaneRow.vue'
import ToggleSwitch from './ToggleSwitch.vue'
import { humanizeMinutes } from '../utils/duration'

/**
 * The frame of a "When the host is offline" section: its heading, a failed
 * load in its place, the read-only summary and the edit form's shell. Shared
 * by the agent's, the repository host's and the dependency's sections. The
 * switch is the same on each, though what it means is said differently (the
 * `intermittentHelp` slot); what is nested under it (the `edit` slot) and what
 * waits on the host (the default slot, under the settings) are the caller's.
 */
const props = defineProps<{
  loadError: string | null
  /** What is saved; `catch_up_recheck_minutes` is `null` for an agent, which is never asked. */
  settings: {
    intermittent: boolean
    catch_up_recheck_minutes: number | null
    catch_up_give_up_minutes: number
  } | null
  /** The sentence under the summary, for a marked and an unmarked host. */
  markedHint: string
  unmarkedHint: string
  lede?: string
  ledeLabel?: string
  editing: boolean
  canEdit: boolean
  saving: boolean
  error: string | null
  /** `HelpHint` label for the switch, e.g. "what an unreachable host means". */
  intermittentHelpLabel: string
}>()

/** The switch's value while editing. */
const intermittent = defineModel<boolean>('intermittent', { required: true })

const emit = defineEmits<{ edit: []; cancel: []; save: [] }>()

const giveUpText = computed(() => {
  const minutes = props.settings?.catch_up_give_up_minutes ?? 0
  return minutes === 0 ? 'Never' : humanizeMinutes(minutes)
})
</script>

<template>
  <section class="pane-section">
    <template v-if="loadError">
      <p class="group-label">When the host is offline</p>
      <div class="state-msg state-msg--inline state-error">
        {{ loadError }}
      </div>
    </template>

    <EditableSection
      v-else-if="settings"
      label="When the host is offline"
      :lede="lede"
      :lede-label="ledeLabel"
      :editing="editing"
      :can-edit="canEdit"
      :saving="saving"
      :error="error"
      @edit="emit('edit')"
      @cancel="emit('cancel')"
      @save="emit('save')"
    >
      <template #view>
        <dl class="info-grid">
          <dt>Host is not always online</dt>
          <dd>{{ settings.intermittent ? 'Yes' : 'No' }}</dd>
          <template v-if="settings.intermittent">
            <template v-if="settings.catch_up_recheck_minutes !== null">
              <dt>Re-check every</dt>
              <dd>{{ humanizeMinutes(settings.catch_up_recheck_minutes) }}</dd>
            </template>
            <dt>Stop waiting after</dt>
            <dd>{{ giveUpText }}</dd>
          </template>
        </dl>
        <p class="field-hint">
          {{ settings.intermittent ? markedHint : unmarkedHint }}
        </p>
      </template>

      <template #edit>
        <div class="pane-rows">
          <PaneRow
            title="Host is not always online"
            :help="intermittentHelpLabel"
          >
            <template #help>
              <slot name="intermittentHelp" />
            </template>
            <ToggleSwitch
              v-model="intermittent"
              label="Host is not always online"
            />
          </PaneRow>
          <slot
            v-if="intermittent"
            name="edit"
          />
        </div>
      </template>
    </EditableSection>

    <slot />
  </section>
</template>
