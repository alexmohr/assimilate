// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { ArchiveEntryResponse } from '../types/generated'

/** Only the two fields the rule reads, so callers can pass a group key too. */
type HostSource = Pick<ArchiveEntryResponse, 'matched' | 'agent_hostname' | 'hostname'>

/**
 * Which hostname an archive belongs to.
 *
 * `agent_hostname` is only meaningful once the archive has been matched to an
 * agent. On an unmatched archive it is whichever agent the archive name
 * happened to resemble, not where the backup came from, so the hostname borg
 * itself recorded is the honest answer - and the only one that should be shown
 * or linked.
 *
 * This lives in one place because it was written three times - the row, the
 * file browser and the grouping key - and the third copy silently dropped the
 * `matched` guard, which would have labelled an archive with the warning stripe
 * and a link to a specific wrong agent at the same time.
 */
export function resolveArchiveHost(archive: HostSource): string {
  return archive.matched === true ? (archive.agent_hostname ?? archive.hostname) : archive.hostname
}
