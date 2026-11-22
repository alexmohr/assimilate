// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { ref, type Ref } from 'vue'
import { createEnrollmentToken, revokeEnrollmentToken } from '../api/enrollment'
import { extractError } from '../utils/error'
import type { CreateEnrollmentTokenResponse } from '../types/generated'

/** How long a token works when the form is left alone: one hour. */
export const DEFAULT_LIFETIME_SECS = 3600

/** The lifetimes the form offers, in seconds. */
export const LIFETIMES: readonly { value: number; label: string }[] = [
  { value: 900, label: '15 minutes' },
  { value: DEFAULT_LIFETIME_SECS, label: '1 hour' },
  { value: 86_400, label: '24 hours' },
]

export interface InstallCommandState {
  lifetime: Ref<number>
  hostname: Ref<string>
  tagIds: Ref<number[]>
  /** The token just created, until the dialog closes. */
  created: Ref<CreateEnrollmentTokenResponse | null>
  loading: Ref<boolean>
  error: Ref<string | null>
  /** Counts up whenever the list of tokens should be read again. */
  version: Ref<number>
  generate: () => Promise<void>
  /** Revokes the token just created, if a host has not used it yet. */
  revoke: () => Promise<void>
  reset: () => void
}

/** The form and the token behind the Add agent dialog's Install command tab. */
export function useInstallCommand(): InstallCommandState {
  const lifetime = ref(DEFAULT_LIFETIME_SECS)
  const hostname = ref('')
  const tagIds = ref<number[]>([])
  const created = ref<CreateEnrollmentTokenResponse | null>(null)
  const loading = ref(false)
  const error = ref<string | null>(null)
  const version = ref(0)

  async function generate(): Promise<void> {
    loading.value = true
    error.value = null
    try {
      created.value = await createEnrollmentToken({
        lifetime_secs: lifetime.value,
        hostname: hostname.value.trim() || undefined,
        tag_ids: tagIds.value,
      })
      version.value++
    } catch (e: unknown) {
      error.value = extractError(e, 'Failed to create the token')
    } finally {
      loading.value = false
    }
  }

  async function revoke(): Promise<void> {
    const token = created.value?.token
    if (!token) return
    try {
      await revokeEnrollmentToken(token.id)
    } catch (e: unknown) {
      // A host may have used it, or it ran out, between creating it and now.
      error.value = extractError(e, 'Failed to revoke the token')
    }
    version.value++
  }

  function reset(): void {
    lifetime.value = DEFAULT_LIFETIME_SECS
    hostname.value = ''
    tagIds.value = []
    created.value = null
    error.value = null
  }

  return { lifetime, hostname, tagIds, created, loading, error, version, generate, revoke, reset }
}
