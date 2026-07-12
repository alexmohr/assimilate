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


def get_failing_check_logs(repo: str, number: int, max_chars: int = 12000) -> str:
    """Best-effort: find failed check runs on the PR and pull their failed-step logs."""
    checks = _run_json(
        [
            "gh",
            "pr",
            "checks",
            str(number),
            "--repo",
            repo,
            "--json",
            "name,state,link",
            "--fail-fast",
        ]
    )
    logs: list[str] = []
    seen_runs: set[str] = set()
    for check in checks:
        if check.get("state") not in ("FAILURE", "ERROR", "CANCELLED"):
            continue
        link = check.get("link") or ""
        m = RUN_ID_RE.search(link)
        if not m:
            continue
        run_id = m.group(1)
        if run_id in seen_runs:
            continue
        seen_runs.add(run_id)
        try:
            out = _run(["gh", "run", "view", run_id, "--repo", repo, "--log-failed"], timeout=180)
        except GhError as exc:
            out = f"(could not fetch log for run {run_id}: {exc})"
        logs.append(f"=== {check.get('name')} (run {run_id}) ===\n{out}")
    combined = "\n\n".join(logs)
    if len(combined) > max_chars:
        combined = combined[-max_chars:]
        combined = "...(truncated)...\n" + combined
    return combined or "(no failed check logs could be retrieved; inspect `gh pr checks` manually)"


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
