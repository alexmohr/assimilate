// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { BadgeTone } from './badge'

/**
 * The sections of a user's page, in order. Kept out of the component so the
 * view can parse `?section=` into the union before handing it down, rather
 * than passing a wide `string` around. Mirrors `dependencyHostSettings.ts`.
 */
export const USER_SECTIONS = ['role', 'access', 'permissions', 'password'] as const

export type UserSection = (typeof USER_SECTIONS)[number]

export function isUserSection(value: unknown): value is UserSection {
  return USER_SECTIONS.some((section) => section === value)
}

/** The two built-in roles a user's primary role is picked from. */
export const BASE_ROLES = ['user', 'admin'] as const

export type BaseRole = (typeof BASE_ROLES)[number]

/**
 * Parses the server's role field. It is an open RBAC name and may list several
 * roles comma-joined ("admin,operator"); holding "admin" anywhere in it is
 * what makes someone an admin, so that is what decides the base role.
 */
export function baseRoleOf(role: string): BaseRole {
  return role.split(',').some((name) => name.trim() === 'admin') ? 'admin' : 'user'
}

export function baseRoleLabel(role: BaseRole): string {
  return role === 'admin' ? 'Admin' : 'User'
}

/** Admins stand out in a list of users; every other role is classification. */
export function roleBadgeTone(role: string): BadgeTone {
  return baseRoleOf(role) === 'admin' ? 'accent' : 'neutral'
}

/** The shortest password the create and reset forms accept. */
export const MIN_PASSWORD_LENGTH = 8
