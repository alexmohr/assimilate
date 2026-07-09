<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

# Review Skill

Use when:

* reviewing a pull request
* responding to review comments on your own PR

## Required

* If not stated otherwise, pull requests should always be squashed during merge.
* Reviews must be done with maximum strictness; findings are never optional.
* Logic is never to be duplicated instead of reused — flag any duplication found.
* New code shall have test coverage.
* User-facing functions should be tested via e2e tests.
* Use GitHub's review and changes-requested functions for all reviews.
* Set the `changes requested` label when something is found, and `ready to merge` once the PR looks good.
* If a PR is behind the main branch, this must be flagged as changes requested. Only a fully rebased branch is acceptable.
* When a comment is addressed, the agent shall reply with "solved please re-review" and leave the comment unresolved.
* The agent that addresses a review comment must never resolve it — only the comment author resolves it.

## Workflow

1. Check whether the PR is rebased on the latest default branch. If not, request changes citing that alone as a finding.
2. Review the diff for correctness, duplicated logic, and test coverage (unit + e2e for user-facing functions) per the Required rules above.
3. Post findings via GitHub's native review tools (inline comments / review body), not as free-form chat replies.
4. Apply the `changes requested` label if any finding exists; apply `ready to merge` once none remain.
5. When changes are pushed addressing a specific comment, reply "solved please re-review" on that comment and leave it unresolved — do not resolve it yourself.

## Validation checklist

* [ ] Rebase status checked and flagged if stale
* [ ] Every finding filed as an actual GitHub review comment, not just prose
* [ ] Duplicated logic called out
* [ ] Test coverage (unit + e2e where user-facing) checked
* [ ] Correct label applied (`changes requested` or `ready to merge`)
* [ ] Own comments never self-resolved; "solved please re-review" used instead
