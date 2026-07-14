# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr

"""Thin wrapper around the `gh` CLI.

Everything here is read-only or additive (comments, our own harness-owned
label) except `create_pr`. The harness never calls anything that would add or
remove the repo's own status labels (`needs review`, `ready to merge`,
`changes requested`, `ci failing`, `merge conflict`, `precheck failed`,
`needs human review`) or the `claude-approved`/`claude-changes-requested`
verdict labels - see skills/review/SKILL.md: "agents must never add or
remove the status labels themselves". Those are owned end-to-end by
.github/workflows/pr-status-labels.yml.
"""

from __future__ import annotations

import json
import logging
import re
import subprocess
import urllib.parse
from dataclasses import dataclass, field
from typing import Any

log = logging.getLogger("harness.gh")

RUN_ID_RE = re.compile(r"/actions/runs/(\d+)")
JOB_ID_RE = re.compile(r"/job/(\d+)")

STATUS_LABEL_NAMES = {
    "needs review",
    "changes requested",
    "ci failing",
    "merge conflict",
    "precheck failed",
    "ready to merge",
    "needs human review",
    "coverage failed",
    "duplicate code",
    "claude-approved",
    "claude-changes-requested",
    "claude review failed",
}


class GhError(RuntimeError):
    pass


def _run(args: list[str], input_text: str | None = None, timeout: int = 120) -> str:
    proc = subprocess.run(
        args,
        input=input_text,
        capture_output=True,
        text=True,
        timeout=timeout,
    )
    if proc.returncode != 0:
        raise GhError(f"{' '.join(args)} failed ({proc.returncode}): {proc.stderr.strip()}")
    return proc.stdout


def _run_json(args: list[str], timeout: int = 120) -> Any:
    return json.loads(_run(args, timeout=timeout))


@dataclass
class PrSummary:
    number: int
    title: str
    head_ref_name: str
    is_draft: bool
    created_at: str
    labels: list[str] = field(default_factory=list)


@dataclass
class PrDetail:
    number: int
    state: str
    title: str
    body: str
    head_ref_name: str
    is_draft: bool
    labels: list[str]
    review_decision: str | None
    merge_state_status: str | None
    status_check_rollup: list[dict[str, Any]]

    @property
    def ci_failing(self) -> bool:
        return "ci failing" in self.labels

    @property
    def merge_conflict(self) -> bool:
        return "merge conflict" in self.labels or self.merge_state_status == "DIRTY"

    @property
    def coverage_failed(self) -> bool:
        return "coverage failed" in self.labels

    @property
    def duplicate_code(self) -> bool:
        return "duplicate code" in self.labels

    @property
    def changes_requested(self) -> bool:
        return (
            self.review_decision == "CHANGES_REQUESTED" or "claude-changes-requested" in self.labels
        )

    @property
    def needs_human_review(self) -> bool:
        return "needs human review" in self.labels

    @property
    def needs_fix(self) -> bool:
        return (
            self.ci_failing
            or self.merge_conflict
            or self.coverage_failed
            or self.duplicate_code
            or self.changes_requested
        )


PR_LIST_FIELDS = "number,title,headRefName,isDraft,createdAt,labels"
PR_VIEW_FIELDS = (
    "number,state,title,body,headRefName,headRepositoryOwner,labels,"
    "reviewDecision,mergeStateStatus,statusCheckRollup"
)


def list_open_prs(repo: str) -> list[PrSummary]:
    raw = _run_json(
        [
            "gh",
            "pr",
            "list",
            "--repo",
            repo,
            "--state",
            "open",
            "--json",
            PR_LIST_FIELDS,
            "--limit",
            "200",
        ]
    )
    prs = [
        PrSummary(
            number=p["number"],
            title=p["title"],
            head_ref_name=p["headRefName"],
            is_draft=p["isDraft"],
            created_at=p["createdAt"],
            labels=[label_name(lbl) for lbl in p.get("labels", [])],
        )
        for p in raw
    ]
    prs.sort(key=lambda p: p.created_at)
    return prs


def label_name(label: Any) -> str:
    return label["name"] if isinstance(label, dict) else str(label)


def get_pr(repo: str, number: int) -> PrDetail:
    raw = _run_json(["gh", "pr", "view", str(number), "--repo", repo, "--json", PR_VIEW_FIELDS])
    return PrDetail(
        number=raw["number"],
        state=raw["state"],
        title=raw["title"],
        body=raw.get("body") or "",
        head_ref_name=raw["headRefName"],
        is_draft=raw.get("isDraft", False),
        labels=[label_name(lbl) for lbl in raw.get("labels", [])],
        review_decision=raw.get("reviewDecision"),
        merge_state_status=raw.get("mergeStateStatus"),
        status_check_rollup=raw.get("statusCheckRollup") or [],
    )


def get_pr_head_sha(repo: str, number: int) -> str:
    raw = _run_json(["gh", "pr", "view", str(number), "--repo", repo, "--json", "commits"])
    commits = raw.get("commits") or []
    return commits[-1]["oid"] if commits else ""


_FAILING_CHECK_CONCLUSIONS = {"failure", "cancelled", "timed_out", "action_required"}


def get_failing_checks(repo: str, number: int) -> list[dict[str, Any]]:
    """Failed/errored/cancelled check runs on the PR's head commit (name, link).

    Deliberately `gh api` (the REST check-runs endpoint) rather than
    `gh pr checks --json`: the latter's `--json` flag doesn't exist on older
    `gh` versions at all ("unknown flag: --json"), which isn't something
    this harness can assume is unavailable on whatever machine runs it - the
    same reasoning as add_label/remove_label using `gh api` instead of
    `gh pr edit` elsewhere in this file.
    """
    head_sha = get_pr_head_sha(repo, number)
    if not head_sha:
        return []
    data = _run_json(["gh", "api", f"repos/{repo}/commits/{head_sha}/check-runs?per_page=100"])
    return [
        {"name": c["name"], "link": c.get("details_url") or c.get("html_url") or ""}
        for c in data.get("check_runs", [])
        if c.get("status") == "completed" and c.get("conclusion") in _FAILING_CHECK_CONCLUSIONS
    ]


def get_failing_check_names(repo: str, number: int) -> list[str]:
    return [c["name"] for c in get_failing_checks(repo, number)]


def get_failing_check_logs(repo: str, number: int, max_chars: int = 12000) -> str:
    """Best-effort: find failed check runs on the PR and pull their failed-step logs.

    Each run's log is truncated to its own fair share of `max_chars` before
    concatenating - not the combined string's tail as a whole. A single
    verbose failing job (e.g. cargo-deny dumping its entire resolved
    dependency tree before the actual advisory line) can otherwise consume
    the whole budget and silently push every other failing check's log out
    of what opencode ever sees, even though that other check might be the
    one with the actually actionable content.
    """
    seen_jobs: set[str] = set()
    jobs: list[tuple[str, str, str | None]] = []  # (name, run_id, job_id)
    for check in get_failing_checks(repo, number):
        link = check.get("link") or ""
        run_m = RUN_ID_RE.search(link)
        if not run_m:
            continue
        job_m = JOB_ID_RE.search(link)
        dedupe_key = job_m.group(1) if job_m else run_m.group(1)
        if dedupe_key in seen_jobs:
            continue
        seen_jobs.add(dedupe_key)
        jobs.append((check.get("name") or "?", run_m.group(1), job_m.group(1) if job_m else None))

    per_job_budget = max(max_chars // max(len(jobs), 1), 2000)
    logs: list[str] = []
    for name, run_id, job_id in jobs:
        # Prefer --job <job_id>: it scopes gh's log fetch to exactly this
        # check, whereas a bare run_id makes gh aggregate every job in the
        # run and filter, which has been observed to come back with an
        # empty log for a specific failing job on a run with many parallel
        # jobs (this repo's CI has ~20). A run-wide, unscoped fetch also
        # means one huge job's failed-step output can dominate what `gh`
        # returns before it ever gets to a smaller, more relevant one.
        cmd = (
            ["gh", "run", "view", "--job", job_id, "--repo", repo, "--log-failed"]
            if job_id
            else ["gh", "run", "view", run_id, "--repo", repo, "--log-failed"]
        )
        try:
            out = _run(cmd, timeout=180)
        except GhError as exc:
            out = f"(could not fetch log for run {run_id}: {exc})"
        if not out.strip():
            out = "(no failed-step log content returned for this check; inspect it on GitHub)"
        if len(out) > per_job_budget:
            out = "...(truncated)...\n" + out[-per_job_budget:]
        logs.append(f"=== {name} (run {run_id}) ===\n{out}")
    return "\n\n".join(logs) or (
        "(no failed check logs could be retrieved; inspect `gh pr checks` manually)"
    )


def get_review_comments(repo: str, number: int, max_chars: int = 8000) -> str:
    """Inline review comments plus top-level review bodies requesting changes."""
    reviews = _run_json(["gh", "api", f"repos/{repo}/pulls/{number}/reviews"])
    inline = _run_json(["gh", "api", f"repos/{repo}/pulls/{number}/comments"])
    parts: list[str] = []
    for r in reviews:
        if r.get("state") == "CHANGES_REQUESTED" and (r.get("body") or "").strip():
            parts.append(f"[review by {r.get('user', {}).get('login')}] {r['body']}")
    for c in inline:
        if (c.get("body") or "").strip():
            path = c.get("path", "?")
            line = c.get("line") or c.get("original_line") or "?"
            parts.append(f"[{path}:{line}] {c['body']}")
    combined = "\n\n".join(parts)
    if len(combined) > max_chars:
        combined = combined[:max_chars] + "\n...(truncated)..."
    return combined or (
        "(review decision is CHANGES_REQUESTED but no comment bodies were found; "
        "check the PR manually)"
    )


def get_bot_comments(
    repo: str, number: int, since_head_sha: str | None = None, max_chars: int = 8000
) -> str:
    """Recent github-actions[bot] issue comments (coverage-diff / duplicate-code findings)."""
    comments = _run_json(["gh", "api", f"repos/{repo}/issues/{number}/comments", "--paginate"])
    parts = [
        c["body"]
        for c in comments
        if c.get("user", {}).get("login", "").endswith("[bot]") and (c.get("body") or "").strip()
    ]
    combined = "\n\n---\n\n".join(parts[-5:])
    if len(combined) > max_chars:
        combined = combined[-max_chars:]
    return combined or (
        "(no bot comments found; check the `coverage failed`/`duplicate code` check runs manually)"
    )


def add_label(repo: str, number: int, label: str) -> None:
    # Deliberately `gh api` (REST), not `gh pr edit --add-label`: the latter's
    # underlying GraphQL query fetches the PR's projectCards field, which
    # GitHub has deprecated/removed, so it fails outright on repos that hit
    # that field regardless of the label mutation itself.
    _run(["gh", "api", f"repos/{repo}/issues/{number}/labels", "-f", f"labels[]={label}"])


def remove_label(repo: str, number: int, label: str) -> None:
    encoded = urllib.parse.quote(label, safe="")
    try:
        _run(["gh", "api", "--method", "DELETE", f"repos/{repo}/issues/{number}/labels/{encoded}"])
    except GhError as exc:
        if "not found" not in str(exc).lower() and "404" not in str(exc):
            raise


def comment(repo: str, number: int, body: str, issue: bool = False) -> None:
    kind = "issue" if issue else "pr"
    _run(["gh", kind, "comment", str(number), "--repo", repo, "--body-file", "-"], input_text=body)


def list_open_issues(repo: str) -> list[dict[str, Any]]:
    raw = _run_json(
        [
            "gh",
            "issue",
            "list",
            "--repo",
            repo,
            "--state",
            "open",
            "--json",
            "number,title,body,createdAt,labels",
            "--limit",
            "200",
        ]
    )
    for issue in raw:
        issue["labels"] = [label_name(lbl) for lbl in issue.get("labels", [])]
    raw.sort(key=lambda i: i["createdAt"], reverse=True)
    return raw


def get_issue(repo: str, number: int) -> dict[str, Any]:
    fields = "number,title,body,state,labels"
    raw = _run_json(["gh", "issue", "view", str(number), "--repo", repo, "--json", fields])
    raw["labels"] = [label_name(lbl) for lbl in raw.get("labels", [])]
    return raw


def create_pr(repo: str, branch: str, base: str, title: str, body: str) -> str:
    out = _run(
        [
            "gh",
            "pr",
            "create",
            "--repo",
            repo,
            "--head",
            branch,
            "--base",
            base,
            "--title",
            title,
            "--body-file",
            "-",
        ],
        input_text=body,
    )
    return out.strip().splitlines()[-1] if out.strip() else ""
