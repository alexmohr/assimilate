<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { deleteRepoHost, getRepoHost, type RepoHost } from '../api/repoHosts'
import { repoHostAvailabilityApi } from '../api/availability'
import { useAsyncAction } from '../composables/useAsyncAction'
import { useWebSocket } from '../composables/useWebSocket'
import { extractError } from '../utils/error'
import { logger } from '../utils/logger'
import { isRepoHostSection, type RepoHostSection } from '../utils/repoHostSettings'
import BaseSpinner from '../components/BaseSpinner.vue'
import ConfirmDeleteDialog from '../components/ConfirmDeleteDialog.vue'
import DetailHeader from '../components/DetailHeader.vue'
import HostAvailabilityCard from '../components/HostAvailabilityCard.vue'
import RepoHostConnectionCard from '../components/RepoHostConnectionCard.vue'
import RepoHostPowerCard from '../components/RepoHostPowerCard.vue'
import SettingsRail, { type SettingsSections } from '../components/SettingsRail.vue'

/**
 * One repository host: the machine borg writes to, and everything that is a
 * fact about it rather than about one repository on it - where it is reached,
 * the SSH host key it presents, how it is woken and whether it is expected to
 * be online. Set here once, for every repository it holds.
 *
 * Admin only, like everything the host's endpoints serve: its repository list
 * names every repository on the machine whoever may see which.
 */
const props = defineProps<{ id: string }>()
const route = useRoute()
const router = useRouter()

const hostId = computed(() => Number(props.id))
const host = ref<RepoHost | null>(null)
const { loading, error, run } = useAsyncAction()

const section = computed<RepoHostSection>({
  get() {
    const s = route.query.section
    return isRepoHostSection(s) ? s : 'connection'
  },
  set(val: RepoHostSection) {
    router.replace({ query: { ...route.query, section: val } })
  },
})

const sections: SettingsSections<RepoHostSection> = [
  { id: 'connection', label: 'Connection' },
  { id: 'power', label: 'Power' },
  { id: 'repositories', label: 'Repositories' },
  { id: 'danger', label: 'Danger zone', danger: true },
]

const repoCountText = computed(() => {
  const count = host.value?.repositories.length ?? 0
  return count === 1 ? '1 repository' : `${count} repositories`
})

async function load(): Promise<void> {
  await run(async () => {
    host.value = await getRepoHost(hostId.value)
  })
}

async function refresh(): Promise<void> {
  try {
    host.value = await getRepoHost(hostId.value)
  } catch (e: unknown) {
    logger.error('background repository host refresh failed', e)
  }
}

const { onMessage } = useWebSocket()
onMessage('DataChanged', () => {
  void refresh()
})

watch(
  () => props.id,
  async () => {
    host.value = null
    await load()
  },
)
onMounted(load)

const showDelete = ref(false)
const deleting = ref(false)
const deleteError = ref<string | null>(null)

async function confirmDelete(): Promise<void> {
  deleting.value = true
  deleteError.value = null
  try {
    await deleteRepoHost(hostId.value)
    showDelete.value = false
    await router.push('/repos')
  } catch (e: unknown) {
    deleteError.value = extractError(e)
  } finally {
    deleting.value = false
  }
}
</script>

<template>
  <div class="repo-host-detail">
    <nav class="detail-breadcrumb">
      <RouterLink
        to="/repos"
        class="crumb-link"
      >
        Repositories
      </RouterLink>
      <span class="crumb-sep">/</span>
      <span class="crumb-current mono">{{ host?.ssh_host ?? '...' }}</span>
    </nav>

    <BaseSpinner
      v-if="loading"
      size="lg"
    />
    <div
      v-else-if="error && !host"
      class="error-banner"
    >
      {{ error }}
    </div>

    <template v-else-if="host">
      <DetailHeader
        :name="host.ssh_host"
        mono
        subtitle="Repository host"
      >
        <template #badges>
          <span
            class="badge"
            :class="host.intermittent ? 'badge--warning' : 'badge--neutral'"
          >
            {{ host.intermittent ? 'Not always online' : 'Always online' }}
          </span>
        </template>
        <template #meta>
          <span>
            port <b>{{ host.ssh_port }}</b>
          </span>
          <span>
            <b>{{ repoCountText }}</b>
          </span>
        </template>
      </DetailHeader>

      <div class="tab-content fade-in">
        <SettingsRail
          v-slot="{ section: current }"
          :sections="sections"
          :section="section"
          label="Repository host sections"
          @update:section="section = $event"
        >
          <RepoHostConnectionCard
            v-if="current === 'connection'"
            :host="host"
            can-edit
            @saved="host = $event"
          />

          <template v-else-if="current === 'power'">
            <RepoHostPowerCard
              :host="host"
              is-admin
              @saved="refresh"
            />
            <HostAvailabilityCard
              :api="repoHostAvailabilityApi(host.id)"
              can-edit
            />
          </template>

          <template v-else-if="current === 'repositories'">
            <p
              v-if="host.repositories.length === 0"
              class="state-msg state-msg--inline"
            >
              No repository uses this host any more.
            </p>
            <div
              v-else
              class="rows"
            >
              <div
                v-for="repo in host.repositories"
                :key="repo.id"
                class="agent-row"
              >
                <i
                  class="agent-row-stripe"
                  aria-hidden="true"
                />
                <RouterLink
                  class="agent-row-name"
                  :to="`/repos/${repo.id}`"
                >
                  {{ repo.name }}
                </RouterLink>
                <span class="agent-row-stats mono">{{ repo.ssh_user }} · {{ repo.repo_path }}</span>
              </div>
            </div>
          </template>

          <template v-else-if="current === 'danger'">
            <div class="danger-body">
              <div class="danger-info">
                <span class="danger-heading">Remove repository host</span>
                <span class="danger-desc">
                  Forget this host and its settings. Only possible once no repository uses it: move
                  or remove them first.
                </span>
              </div>
              <button
                class="btn btn-sm btn-danger"
                type="button"
                :disabled="host.repositories.length > 0"
                @click="showDelete = true"
              >
                Remove host
              </button>
            </div>
          </template>
        </SettingsRail>
      </div>

      <ConfirmDeleteDialog
        :show="showDelete"
        title="Remove repository host"
        :submitting="deleting"
        :error="deleteError"
        confirm-label="Remove"
        submitting-label="Removing..."
        @cancel="showDelete = false"
        @confirm="confirmDelete"
      >
        Remove <span class="mono">{{ host.ssh_host }}</span> and its settings?
      </ConfirmDeleteDialog>
    </template>
  </div>
</template>

<style scoped>
.repo-host-detail {
  max-width: 1200px;
}
</style>
