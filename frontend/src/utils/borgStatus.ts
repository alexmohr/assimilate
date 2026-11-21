// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { AgentResponse, BorgStatus } from '../types/generated'

/** What is wrong with a host's borg, in the words the UI shows. */
export interface BorgProblem {
  /** The chip on the hosts list. */
  chip: string
  /** The banner's heading on the host's page. */
  heading: string
  /** The sentence under it. */
  detail: string
}

/**
 * What stops backups on a host, or `null` when borg is usable or the host has
 * said nothing (offline, or an agent older than the check).
 */
export function borgProblem(status: BorgStatus | null | undefined): BorgProblem | null {
  if (!status) return null
  switch (status.state) {
    case 'available':
      return null
    case 'missing':
      return {
        chip: 'Borg not installed',
        heading: 'Borg is not installed on this host',
        detail: `The agent looked for ${status.binary} and did not find it. Backups for this host will fail until it is installed.`,
      }
    case 'broken':
      return {
        chip: 'Borg not working',
        heading: 'Borg does not run on this host',
        detail: `The agent could not get a version from ${status.binary}: ${status.reason}. Backups for this host will fail until that is fixed.`,
      }
  }
}

/** The borg version a host reported, if it has a working borg. */
export function borgVersion(agent: Pick<AgentResponse, 'borg'>): string | null {
  return agent.borg?.state === 'available' ? agent.borg.version : null
}
