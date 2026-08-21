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
