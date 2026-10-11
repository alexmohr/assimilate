<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { getRepoHostDependencies, updateRepoHostDependencies } from '../api/repoHosts'
import RequiredDependenciesCard from './RequiredDependenciesCard.vue'

/**
 * The dependencies every backup writing to this repository host needs - for
 * a host whose storage is itself a share on another machine, so every
 * schedule writing here waits for that machine too.
 */
const props = defineProps<{
  hostId: number
  canEdit: boolean
}>()
</script>

<template>
  <RequiredDependenciesCard
    :subject="`repo-host:${hostId}`"
    :load="() => getRepoHostDependencies(props.hostId)"
    :save="(ids) => updateRepoHostDependencies(props.hostId, ids)"
    :can-edit="canEdit"
    lede="Machines every backup writing to this host needs - say, the NAS its storage lives on."
    scope-hint="Applies to every schedule writing to a repository on this host, for each of its agents."
  />
</template>
