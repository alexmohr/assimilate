// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { ReportResponse } from './generated'

export type ReportRow = Omit<ReportResponse, 'schedule_id'> & {
  schedule_id: number | null
}
