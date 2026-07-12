# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr

"""Small persisted JSON state file so the harness survives restarts.

Tracks, per PR/issue number, the fingerprint of the last problem the harness
tried to fix and how many consecutive cycles that exact fingerprint has
survived a push. This is the circuit breaker: it is what lets the harness
tell "still fixing the same thing" apart from "fixed one thing, now facing a
new one" without needing an in-memory process that never restarts.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any


@dataclass
class PrAttempt:
    fingerprint: str = ""
    attempts: int = 0
    last_head_sha: str = ""


@dataclass
class HarnessState:
    path: Path
    pr_attempts: dict[str, PrAttempt] = field(default_factory=dict)
    started_issue_numbers: list[int] = field(default_factory=list)

    @staticmethod
    def load(path: Path) -> HarnessState:
        if not path.exists():
            return HarnessState(path=path)
        raw: dict[str, Any] = json.loads(path.read_text())
        pr_attempts = {
            number: PrAttempt(**data) for number, data in raw.get("pr_attempts", {}).items()
        }
        return HarnessState(
            path=path,
            pr_attempts=pr_attempts,
            started_issue_numbers=list(raw.get("started_issue_numbers", [])),
        )

    def save(self) -> None:
        self.path.parent.mkdir(parents=True, exist_ok=True)
        payload = {
            "pr_attempts": {
                number: {
                    "fingerprint": a.fingerprint,
                    "attempts": a.attempts,
                    "last_head_sha": a.last_head_sha,
                }
                for number, a in self.pr_attempts.items()
            },
            "started_issue_numbers": self.started_issue_numbers,
        }
        self.path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n")

    def record_attempt(self, pr_number: int, fingerprint: str, head_sha: str) -> int:
        """Record a fix attempt against `fingerprint`, returning the attempt count.

        Resets to 1 if the fingerprint differs from the last recorded one for
        this PR (a genuinely new problem), otherwise increments.
        """
        key = str(pr_number)
        existing = self.pr_attempts.get(key)
        if existing is not None and existing.fingerprint == fingerprint:
            existing.attempts += 1
            existing.last_head_sha = head_sha
            self.save()
            return existing.attempts
        self.pr_attempts[key] = PrAttempt(
            fingerprint=fingerprint, attempts=1, last_head_sha=head_sha
        )
        self.save()
        return 1

    def clear_pr(self, pr_number: int) -> None:
        self.pr_attempts.pop(str(pr_number), None)
        self.save()

    def mark_issue_started(self, issue_number: int) -> None:
        if issue_number not in self.started_issue_numbers:
            self.started_issue_numbers.append(issue_number)
            self.save()
