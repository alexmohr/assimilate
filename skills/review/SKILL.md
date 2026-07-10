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
* Use GitHub's review and changes-requested functions for all reviews — submit the verdict through GitHub's native review API (approve / request changes), never by hand-editing labels. See "PR status labels" below.
* If a PR is behind the main branch, this must be flagged as changes requested. Only a fully rebased branch is acceptable.
* When a comment is addressed, the agent shall reply with "solved please re-review" and leave the comment unresolved.
* The agent that addresses a review comment must never resolve it — only the comment author resolves it.

## Workflow

1. Check whether the PR is rebased on the latest default branch. If not, request changes citing that alone as a finding.
2. Review the diff for correctness, duplicated logic, and test coverage (unit + e2e for user-facing functions) per the Required rules above.
3. Post findings via GitHub's native review tools (inline comments / review body), not as free-form chat replies.
4. Submit the review as **Request changes** if any finding exists, or **Approve** once none remain. Do not set status labels yourself — the `PR Status Labels` workflow (`.github/workflows/pr-status-labels.yml`) derives them automatically from your review verdict and CI. See below.
5. When changes are pushed addressing a specific comment, reply "solved please re-review" on that comment and leave it unresolved — do not resolve it yourself.

## PR status labels

`.github/workflows/pr-status-labels.yml` keeps a small set of mutually-exclusive
status labels in sync automatically, driven only by two objective signals: the
`CI` workflow's conclusion on the head commit, and GitHub's native review
decision (`pulls.pull_request_review_write` approve/request-changes). It reruns
on every push, every submitted/dismissed review, and every CI completion, so
the label always reflects current reality — **agents must never add or remove
these labels by hand**; changing the underlying signal (push a fix, submit a
review) is the only way to move them.

| Label | Meaning | Set when |
|---|---|---|
| `needs review` | No blocking verdict yet | Default state; also holds while `needs human review` is outstanding |
| `changes requested` | A reviewer requested changes | GitHub review decision is `CHANGES_REQUESTED` |
| `ci failing` | Latest commit's CI run did not succeed | `CI` workflow conclusion is not `success` — always wins, and strips `ready to merge` |
| `ready to merge` | Fully clear to merge | CI conclusion is `success` **and** review decision is `APPROVED` **and** no human sign-off is pending |
| `needs human review` | Requires a human's sign-off before merge, regardless of agent review | Auto-applied when the diff touches security/crypto/auth/SSH-forwarding code, CI/CD workflow files, dependency lockfiles, `deny.toml`, DB migrations, or adds a new `#[allow(...)]`/`deny.toml` `ignore` suppression |

`needs human review` is additive-only: the workflow will add it but will
**never** remove it — only a human clearing the label counts as sign-off. Even
a PR that is fully approved and green stays capped at `needs review` while
this label is present. If your review touches one of the sensitive areas
above, expect this label and do not treat an agent approval as sufficient to
merge.

## Validation checklist

* [ ] Rebase status checked and flagged if stale
* [ ] Every finding filed as an actual GitHub review comment, not just prose
* [ ] Duplicated logic called out
* [ ] Test coverage (unit + e2e where user-facing) checked
* [ ] Review submitted via GitHub's approve/request-changes API (labels are derived automatically — never set by hand)
* [ ] Own comments never self-resolved; "solved please re-review" used instead
