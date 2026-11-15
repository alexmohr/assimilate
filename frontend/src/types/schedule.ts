// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type {
  OnFailure,
  ScheduleResponse,
  ScheduleType as GeneratedScheduleType,
} from './generated'

export type ScheduleType = GeneratedScheduleType
export type ScheduleFailureAction = OnFailure
export type ScheduleRow = ScheduleResponse

/**
 * One repository a schedule writes into, resolved to its name.
 *
 * `ScheduleRepoTarget` (the request/response shape) carries only `repo_id`,
 * and the name it resolves to comes from a separate, permission-filtered
 * repository list - so every screen that shows targets to a *reader* rather
 * than editing them wants this pair, not the raw target.
 */
export interface ScheduleRepoOption {
  id: number
  name: string
  /** A best-effort target (`false`) is reported as a warning and never stops the run. */
  required: boolean
}
