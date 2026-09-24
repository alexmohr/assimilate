// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { AuditEvent } from '../types/generated'

/**
 * Whether an audit entry recorded any details worth showing. Actions that
 * have none (a key export, say) still carry an empty `details` object.
 */
export function hasAuditDetails(event: Pick<AuditEvent, 'details'>): boolean {
  return event.details != null && Object.keys(event.details).length > 0
}
