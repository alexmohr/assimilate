// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { test } from '@playwright/test';
import { getAdminStorageState } from './helpers/auth';

test('save admin storage state', async ({ browser }) => {
  await getAdminStorageState(browser);
});
