# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr

"""Configuration for the opencode harness, sourced from environment variables."""

from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path

from agent_runner import SUPPORTED_CLIS as _VALID_AGENT_CLIS


def _int(name: str, default: int) -> int:
    return int(os.environ.get(name, str(default)))


def _bool(name: str, default: bool) -> bool:
    val = os.environ.get(name)
    if val is None:
        return default
    return val.strip().lower() in ("1", "true", "yes", "on")


def _optional_int(name: str) -> int | None:
    val = os.environ.get(name)
    return int(val) if val else None


# Each backend's own cheap/fast classifier model, used as HARNESS_ROUTER_MODEL's
# default - keyed by agent_cli so switching backends doesn't silently try to
# route through a model id the other CLI's provider doesn't recognize.
_DEFAULT_ROUTER_MODEL_BY_CLI = {
    "opencode": "opencode-go/deepseek-v4-flash",
    "claude": "claude-haiku-4-5-20251001",
}


def _agent_cli(name: str, default: str) -> str:
    val = os.environ.get(name, default).strip().lower()
    if val not in _VALID_AGENT_CLIS:
        raise ValueError(
            f"{name}={val!r} is not a supported agent CLI - must be one of {_VALID_AGENT_CLIS}"
        )
    return val


def default_router_model(agent_cli: str) -> str:
    """The router model HARNESS_ROUTER_MODEL defaults to for `agent_cli` when
    it isn't set explicitly - public so main()'s `--agent-cli` CLI override
    (which lands after Config.from_env() has already resolved a router_model
    from whatever HARNESS_AGENT_CLI was in the environment) can re-derive the
    right default for the CLI-chosen backend instead of silently keeping the
    other backend's router model.
    """
    return _DEFAULT_ROUTER_MODEL_BY_CLI[agent_cli]


@dataclass(frozen=True)
class Config:
    repo: str
    repo_dir: Path
    base_branch: str
    poll_interval_seconds: int
    agent_cli: str
    agent_model: str | None
    router_model: str
    router_timeout_seconds: int
    opencode_timeout_seconds: int
    max_local_validation_attempts: int
    max_stuck_cycles: int
    max_solved: int | None
    target_prs: tuple[int, ...] | None
    target_all_prs: bool
    target_issues: tuple[int, ...] | None
    fallback_to_issues: bool
    stuck_label: str
    question_label: str
    ignore_label: str
    state_file: Path
    log_file: Path | None
    dry_run: bool
    once: bool

    @staticmethod
    def from_env() -> Config:
        repo_dir = Path(os.environ.get("HARNESS_REPO_DIR", ".")).resolve()
        log_file_env = os.environ.get("HARNESS_LOG_FILE")
        agent_cli = _agent_cli("HARNESS_AGENT_CLI", "opencode")
        return Config(
            repo=os.environ.get("HARNESS_REPO", "alexmohr/assimilate"),
            repo_dir=repo_dir,
            base_branch=os.environ.get("HARNESS_BASE_BRANCH", "main"),
            poll_interval_seconds=_int("HARNESS_POLL_INTERVAL", 180),
            agent_cli=agent_cli,
            agent_model=None,
            router_model=os.environ.get(
                "HARNESS_ROUTER_MODEL", _DEFAULT_ROUTER_MODEL_BY_CLI[agent_cli]
            ),
            router_timeout_seconds=_int("HARNESS_ROUTER_TIMEOUT", 120),
            opencode_timeout_seconds=_int("HARNESS_OPENCODE_TIMEOUT", 14400),
            max_local_validation_attempts=_int("HARNESS_MAX_LOCAL_ATTEMPTS", 3),
            max_stuck_cycles=_int("HARNESS_MAX_STUCK_CYCLES", 3),
            max_solved=_optional_int("HARNESS_MAX_SOLVED"),
            target_prs=None,
            target_all_prs=False,
            target_issues=None,
            fallback_to_issues=_bool("HARNESS_FALLBACK_TO_ISSUES", True),
            stuck_label=os.environ.get("HARNESS_STUCK_LABEL", "opencode-harness-stuck"),
            question_label=os.environ.get("HARNESS_QUESTION_LABEL", "opencode-harness-question"),
            ignore_label=os.environ.get("HARNESS_IGNORE_LABEL", "opencode-harness-ignore"),
            state_file=Path(
                os.environ.get(
                    "HARNESS_STATE_FILE",
                    str(repo_dir / "tools" / "opencode-harness" / ".state.json"),
                )
            ).resolve(),
            log_file=Path(log_file_env).resolve() if log_file_env else None,
            dry_run=_bool("HARNESS_DRY_RUN", False),
            once=_bool("HARNESS_ONCE", False),
        )

    def summary(self) -> str:
        """One-line dump of every resolved setting, logged at startup so a
        misconfigured env var (e.g. set on its own line without `export`,
        so it never reached this process) is visible immediately instead of
        only showing up as an unexplained default several log lines later."""
        model = self.agent_model or f"(routed per-task via {self.router_model})"
        max_solved = self.max_solved if self.max_solved is not None else "unlimited"
        target = "auto"
        if self.target_all_prs:
            target = "all open PRs"
        elif self.target_prs is not None:
            target = "pr(s) " + ",".join(f"#{n}" for n in self.target_prs)
        elif self.target_issues is not None:
            target = "issue(s) " + ",".join(f"#{n}" for n in self.target_issues)
        return (
            f"repo={self.repo} repo_dir={self.repo_dir} base_branch={self.base_branch} "
            f"poll_interval={self.poll_interval_seconds}s agent_cli={self.agent_cli} "
            f"model={model} target={target} "
            f"opencode_timeout={self.opencode_timeout_seconds}s "
            f"max_local_attempts={self.max_local_validation_attempts} "
            f"max_stuck_cycles={self.max_stuck_cycles} max_solved={max_solved} "
            f"stuck_label={self.stuck_label} question_label={self.question_label} "
            f"fallback_to_issues={self.fallback_to_issues} "
            f"dry_run={self.dry_run} once={self.once}"
        )
