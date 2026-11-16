// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { HostProvider } from '../types/generated'

/** What each repository host provider is called in the UI. */
export const PROVIDER_LABELS: Record<HostProvider, string> = {
  generic: 'Generic SSH',
  hetzner: 'Hetzner Storage Box',
  rsync_net: 'rsync.net',
  borgbase: 'BorgBase',
}
