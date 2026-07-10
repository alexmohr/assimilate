// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

// Shared "find our marked comment, then create or update it" mechanics for
// the pre-flight check scripts (analyze-duplication.js,
// pre-review-checks.js) - each owns its own HTML-comment marker and message
// text, only the create-or-update plumbing was duplicated between them.

async function upsertMarkedComment(github, owner, repo, prNumber, marker, body, { onlyIfExists = false } = {}) {
  const comments = await github.paginate(github.rest.issues.listComments, {
    owner,
    repo,
    issue_number: prNumber,
    per_page: 100,
  });
  const existing = comments.find((c) => c.body.startsWith(marker));

  if (existing) {
    await github.rest.issues.updateComment({ owner, repo, comment_id: existing.id, body });
  } else if (!onlyIfExists) {
    await github.rest.issues.createComment({ owner, repo, issue_number: prNumber, body });
  }
}

module.exports = { upsertMarkedComment };
