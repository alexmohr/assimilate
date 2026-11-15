// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { AgentResponse, HostnamePatternResponse } from './generated'

/**
 * A host's include patterns, the exceptions to its excludes. Optional until
 * the backend adds them to `AgentResponse`; a schedule has had them all along.
 */
export interface AgentFileSetExtras {
  default_include_patterns?: string[]
}

export type AgentRow = AgentResponse & AgentFileSetExtras

export type AgentHostnamePattern = HostnamePatternResponse
