// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

// Pins the two decisions that stand between "the automation approved it" and
// "the automation merged it": how the AUTO_MERGE_ENABLED kill switch is read,
// and what auto-merge refuses to land on its own.

const assert = require("node:assert/strict");
const { test } = require("node:test");

const fs = require("node:fs");
const path = require("node:path");

const {
  autoMergeBlockedReason,
  autoMergeIfApproved,
  approvalIsCurrent,
  LISTED_FILES_CAP,
  parseAutoMergeEnabled,
  resolveAutoMerge,
  touchesProtectedPaths,
  AUTO_MERGE_PROTECTED_PREFIXES,
  AUTO_MERGE_PROTECTED_CONFIG_DIRS,
  AUTO_MERGE_PROTECTED_BASENAMES,
  AUTO_MERGE_PROTECTED_DOT_TOP_LEVEL,
} = require("../sync-pr-labels.js");

function recordingCore() {
  const warnings = [];
  return { core: { warning: (message) => warnings.push(message) }, warnings };
}

test("kill_switch_defaults_to_enabled_when_unset", () => {
  const { core, warnings } = recordingCore();

  // A workflow expression renders a never-set variable as the empty string,
  // so these are the shapes "unset" actually arrives in.
  assert.equal(parseAutoMergeEnabled(undefined, core), true);
  assert.equal(parseAutoMergeEnabled(null, core), true);
  assert.equal(parseAutoMergeEnabled("", core), true);
  assert.deepEqual(warnings, [], "the documented default is not worth warning about");
});

test("kill_switch_treats_a_blank_but_present_value_as_a_mistake", () => {
  const { core, warnings } = recordingCore();

  // Filling the variable in with blanks is a typo, not a request for the
  // default, so it fails closed and says so - otherwise it reads as "unset"
  // and silently keeps merging.
  assert.equal(parseAutoMergeEnabled(" ", core), false);
  assert.equal(parseAutoMergeEnabled("\t", core), false);
  assert.equal(warnings.length, 2);
  // The raw value, not the trimmed one: `""` in the log would read exactly
  // like the unset case this deliberately is not.
  assert.match(warnings[0], /unrecognised value \(" "\)/);
});

test("kill_switch_recognises_both_value_sets", () => {
  const { core, warnings } = recordingCore();

  for (const on of ["true", "1", "yes", "on", "enabled", "TRUE", "  On  "]) {
    assert.equal(parseAutoMergeEnabled(on, core), true, `${on} should enable`);
  }
  for (const off of ["false", "0", "no", "off", "disabled", "FALSE", "  Off  "]) {
    assert.equal(parseAutoMergeEnabled(off, core), false, `${off} should disable`);
  }
  assert.deepEqual(warnings, []);
});

test("kill_switch_fails_closed_on_an_unrecognised_value", () => {
  const { core, warnings } = recordingCore();

  // The direction that matters: someone reaching for the kill switch and
  // fat-fingering it must not end up merging code unattended in silence.
  assert.equal(parseAutoMergeEnabled("disbaled", core), false);
  assert.equal(warnings.length, 1);
  assert.match(warnings[0], /unrecognised value/);
});

test("auto_merge_refuses_to_land_changes_to_the_rails", () => {
  // The rails are every file that defines or suppresses a CI gate, and each is
  // read from the PR's own head - so editing one takes effect on the very run
  // that decides whether that PR may merge. A PR editing any of them must wait
  // for a person, or the rails could widen themselves on the automation's own
  // approval. The two lists are asserted whole: a path silently dropped from
  // one is a gate that quietly becomes auto-mergeable again.
  assert.deepEqual(AUTO_MERGE_PROTECTED_PREFIXES, [
    ".github/",
    "lints/",
    "frontend/eslint-rules/",
    "scripts/",
  ]);
  assert.deepEqual(AUTO_MERGE_PROTECTED_CONFIG_DIRS, ["", "frontend/"]);
  assert.equal(AUTO_MERGE_PROTECTED_DOT_TOP_LEVEL, true);
  assert.deepEqual(AUTO_MERGE_PROTECTED_BASENAMES, ["Cargo.toml"]);

  for (const filename of [
    // .github/ - the analyzers, the workflows, and this script.
    ".github/scripts/analyze-coverage-diff.js",
    ".github/scripts/lib/lcov.js",
    ".github/scripts/sync-pr-labels.js",
    ".github/workflows/ci.yml",
    ".github/workflows/coverage-diff-check.yml",
    // lints/ and frontend/eslint-rules/ - the same no-string-control-flow rule
    // on each side, both built from the PR's own checkout.
    "lints/no_string_control_flow/src/lib.rs",
    "frontend/eslint-rules/no-string-literal-control-flow.js",
    // scripts/ - the entry points for the repo's local pre-commit hooks.
    "scripts/check-no-raw-sqlx-queries.sh",
    "scripts/no-typography-in-comments.py",
    // Repository-root config: every gate whose settings live at the top level
    // reads them from the PR's head, so loosening one lands green on the same
    // run that loosened it.
    "Cargo.toml",
    "deny.toml",
    ".jscpd.json",
    ".pre-commit-config.yaml",
    "clippy.toml",
    ".rustfmt.toml",
    "mkdocs.yml",
    // frontend/ top-level config: package.json holds the `lint` and `build`
    // scripts ci.yml actually invokes, eslint.config.js wires up the frontend
    // half of the no-string-control-flow rule, and the tsconfigs are what
    // `vue-tsc -b` enforces.
    "frontend/package.json",
    "frontend/package-lock.json",
    "frontend/eslint.config.js",
    "frontend/tsconfig.app.json",
    "frontend/playwright.config.ts",
    "frontend/.npm-audit-allowlist.json",
    // A member crate's Cargo.toml carries `[lints] workspace = true`, the only
    // thing applying the root deny list to that crate. validate-cargo-lints
    // checks the root file against the shared baseline and never checks that
    // members opt in, so dropping those two lines goes green.
    "crates/server/Cargo.toml",
    "crates/agent/Cargo.toml",
    // Root dot-directories. .sqlx is the offline query cache sqlx's macros
    // compile against; .reuse holds the REUSE hook's templates. The two that
    // matter most do not exist yet, because the move is to ADD them:
    // .cargo/config.toml's `[build] rustflags = ["--cap-lints=allow"]` caps
    // every lint rustc-wide and defangs `clippy -- -D warnings` on the run
    // that adds it, and .config/nextest.toml is the same for the test job.
    ".cargo/config.toml",
    ".config/nextest.toml",
    ".sqlx/query-abc.json",
    ".reuse/templates/assimilate.jinja2",
  ]) {
    assert.equal(
      touchesProtectedPaths([{ filename: "src/unrelated.rs" }, { filename }]),
      true,
      `${filename} must block auto-merge`,
    );
  }
});

test("auto_merge_allows_ordinary_changes", () => {
  assert.equal(
    touchesProtectedPaths([
      { filename: "crates/server/src/api/repos.rs" },
      { filename: "frontend/src/views/ReposView.vue" },
      { filename: "docs/backups.md" },
      { filename: "skills/review/SKILL.md" },
    ]),
    false,
  );
  assert.equal(touchesProtectedPaths([]), false);
});

test("auto_merge_guard_sees_a_rename_out_of_the_protected_path", () => {
  // Renaming a gate out of .github/ in the same PR that edits it would
  // otherwise leave only an innocent-looking destination path.
  assert.equal(
    touchesProtectedPaths([
      {
        filename: "tools/analyze-coverage-diff.js",
        previous_filename: ".github/scripts/analyze-coverage-diff.js",
      },
    ]),
    true,
  );
});

test("auto_merge_guard_is_not_fooled_by_a_lookalike_path", () => {
  assert.equal(touchesProtectedPaths([{ filename: "docs/.github/notes.md" }]), false);
  assert.equal(touchesProtectedPaths([{ filename: "vendor/x.github/thing.yml" }]), false);
  assert.equal(touchesProtectedPaths([{ filename: "frontend/scripts/build.ts" }]), false);
  assert.equal(touchesProtectedPaths([{ filename: "crates/agent/lints/notes.md" }]), false);
  // A prefix rule matches from the start of the path, so a nested directory of
  // the same name is not the protected one.
  assert.equal(touchesProtectedPaths([{ filename: "frontend/src/scripts/x.ts" }]), false);
});

test("every_immediate_child_of_a_config_dir_is_protected_not_just_the_gate_files", () => {
  // The rule covers the whole level, not a list of files that configure a gate
  // today - that list is the enumeration this replaced, and the next config
  // file added at either level would land outside it. So ordinary root files
  // block auto-merge too. That over-breadth is the design: it costs a person
  // one click, where an under-broad rule costs the gate. Narrowing the rule
  // back to "known gate files" must fail here rather than pass quietly.
  for (const filename of ["README.md", "LICENSE", "AGENTS.md", "Cargo.lock", "frontend/index.html"]) {
    assert.equal(
      touchesProtectedPaths([{ filename }]),
      true,
      `${filename} is an immediate child of a config directory and must block auto-merge`,
    );
  }
});

test("a_root_dot_directory_added_by_the_pr_itself_is_protected", async () => {
  // The whole point of matching the dot structurally rather than naming
  // directories: neither .cargo/ nor .config/ exists in this repo, so a list
  // of known config directories could not have covered them. Driven through
  // the enforcement point, not just the predicate, because this is the case a
  // PR would actually construct.
  const { github, calls } = fakeGithub([
    { filename: "crates/server/src/lib.rs" },
    { filename: ".cargo/config.toml" },
  ]);
  const messages = [];

  await autoMergeIfApproved(github, { info: (m) => messages.push(m) }, "o", "r", 7, samePrRepo, true);

  assert.deepEqual(calls.merged, [], "a PR adding .cargo/config.toml must not be merged");
  assert.deepEqual(calls.deletedRefs, [], "and its branch must survive");
  assert.match(messages.join("\n"), /it changes \.cargo\/config\.toml/);
});

test("config_protection_stops_at_the_directory_it_names", () => {
  // The config-directory rule covers immediate children only, so ordinary work
  // under a subdirectory of one stays auto-mergeable. Without this the guard
  // would swallow the whole repository and auto-merge would never fire at all.
  for (const filename of [
    "crates/server/src/api/repos.rs",
    "frontend/src/views/ReposView.vue",
    "frontend/e2e/repos.spec.ts",
    "docs/backups.md",
    "skills/review/SKILL.md",
    // The dot rule is top-level only: a dot-directory nested under source is
    // not configuration this repo's CI reads.
    "crates/server/.vscode/settings.json",
  ]) {
    assert.equal(
      touchesProtectedPaths([{ filename }]),
      false,
      `${filename} is ordinary work and must stay auto-mergeable`,
    );
  }
});

test("raw_variable_wins_over_the_boolean_when_the_caller_passes_one", () => {
  const { core, warnings } = recordingCore();

  // The workflows hand over AUTO_MERGE_ENABLED's text; this module parses it.
  assert.equal(resolveAutoMerge({ autoMergeEnabled: false, autoMergeEnabledRaw: "", core }), true);
  assert.equal(
    resolveAutoMerge({ autoMergeEnabled: true, autoMergeEnabledRaw: "false", core }),
    false,
  );
  assert.deepEqual(warnings, []);
});

test("boolean_stands_when_no_raw_variable_is_passed", () => {
  const { core } = recordingCore();

  // pre-review-checks.js's call site: pinned off, no variable involved.
  assert.equal(resolveAutoMerge({ autoMergeEnabled: false, core }), false);
  assert.equal(resolveAutoMerge({ autoMergeEnabled: true, core }), true);
  // A caller that passes neither must get a decision, not `undefined` - and
  // that decision is "do not merge".
  assert.equal(resolveAutoMerge({ core }), false);
});

test("workflows_pass_the_variable_rather_than_calling_across_the_version_seam", () => {
  // Regression test for a real CI failure on this PR: `sync-pr-labels.js` is
  // always checked out from the default branch, but on a
  // `pull_request_review` event the workflow file comes from the PR's head.
  // A workflow body calling `sync.parseAutoMergeEnabled(...)` therefore died
  // with "sync.parseAutoMergeEnabled is not a function" until the PR adding
  // that export reached the default branch. Passing data across that seam
  // survives the skew; calling a function does not.
  const workflows = path.join(__dirname, "..", "..", "workflows");

  for (const file of ["pr-status-labels.yml", "claude-review.yml"]) {
    const body = fs.readFileSync(path.join(workflows, file), "utf8");

    assert.ok(
      !body.includes("sync.parseAutoMergeEnabled"),
      `${file} must not call parseAutoMergeEnabled - the script it loads may predate it`,
    );
    assert.ok(
      body.includes("autoMergeEnabledRaw: process.env.AUTO_MERGE_ENABLED"),
      `${file} should hand the raw variable to sync-pr-labels.js instead`,
    );
  }
});

// `touchesProtectedPaths` above is the predicate; these drive the enforcement
// point itself. An inverted condition or a dropped `return` in
// `autoMergeIfApproved` would merge exactly the changes the guard exists to
// hold back, and no unit test of the predicate would notice. These scripts
// aren't lcov-instrumented either, so the coverage gate can't see this wiring
// - `node --test` is the only net under it.
function fakeGithub(files, { mergeError } = {}) {
  const calls = { merged: [], deletedRefs: [] };
  const github = {
    paginate: async () => files,
    rest: {
      pulls: {
        listFiles: {},
        merge: async (args) => {
          calls.merged.push(args);
          if (mergeError) throw mergeError;
        },
      },
      git: {
        deleteRef: async (args) => {
          calls.deletedRefs.push(args);
          return {};
        },
      },
    },
  };
  return { github, calls };
}

// Same-repo PR, so the branch-delete path is exercised too.
const samePrRepo = {
  head: { ref: "feature", sha: "headsha1", repo: { id: 1 } },
  base: { repo: { id: 1 } },
};

test("auto_merge_skips_a_pr_that_touches_the_rails", async () => {
  const { github, calls } = fakeGithub([
    { filename: "crates/server/src/lib.rs" },
    { filename: ".github/scripts/analyze-coverage-diff.js" },
  ]);
  const messages = [];

  await autoMergeIfApproved(
    github,
    { info: (m) => messages.push(m) },
    "o",
    "r",
    7,
    samePrRepo,
    true,
  );

  assert.deepEqual(calls.merged, [], "a PR changing .github/ must not be merged");
  assert.deepEqual(calls.deletedRefs, [], "and its branch must survive");
  assert.match(messages.join("\n"), /auto-merge deliberately does not land changes/);
});

test("auto_merge_proceeds_for_an_ordinary_pr", async () => {
  const { github, calls } = fakeGithub([
    { filename: "crates/server/src/lib.rs" },
    { filename: "docs/backups.md" },
  ]);

  await autoMergeIfApproved(github, { info: () => {} }, "o", "r", 7, samePrRepo, true);

  assert.equal(calls.merged.length, 1, "an ordinary PR still merges");
  assert.deepEqual(calls.merged[0], {
    owner: "o",
    repo: "r",
    pull_number: 7,
    merge_method: "squash",
    // Pinned to the head the guard above was computed against - see the
    // head-moved test below for why this is load-bearing rather than cosmetic.
    sha: "headsha1",
  });
  assert.equal(calls.deletedRefs.length, 1, "and its same-repo branch is deleted");
  assert.equal(calls.deletedRefs[0].ref, "heads/feature");
});

test("auto_merge_leaves_a_fork_branch_alone", async () => {
  const { github, calls } = fakeGithub([{ filename: "docs/backups.md" }]);
  const forkPr = {
    head: { ref: "feature", sha: "headsha1", repo: { id: 2 } },
    base: { repo: { id: 1 } },
  };

  await autoMergeIfApproved(github, { info: () => {} }, "o", "r", 7, forkPr, true);

  assert.equal(calls.merged.length, 1);
  assert.deepEqual(calls.deletedRefs, [], "this token can't delete a fork's branch");
});

test("a_head_that_moved_after_the_guard_ran_is_not_merged", async () => {
  // The guard reads the PR's file list, then merges. Without `sha` GitHub
  // would merge whatever the head is at merge time, so a commit landing in
  // between would be merged having never been checked against the protected
  // paths - going around the guard rather than through it. Pinning the sha
  // turns that into a 409, which is a no-op: nothing merged, branch intact,
  // no job failure, and the next sync re-evaluates against the new head.
  const conflict = Object.assign(new Error("Head branch was modified"), { status: 409 });
  const { github, calls } = fakeGithub([{ filename: "docs/backups.md" }], {
    mergeError: conflict,
  });
  const messages = [];

  await autoMergeIfApproved(github, { info: (m) => messages.push(m) }, "o", "r", 7, samePrRepo, true);

  assert.equal(calls.merged.length, 1, "the merge was attempted");
  assert.equal(calls.merged[0].sha, "headsha1", "and pinned to the certified head");
  assert.deepEqual(calls.deletedRefs, [], "but nothing merged, so the branch must survive");
  assert.match(messages.join("\n"), /auto-merge attempt skipped \(409\)/);
});

test("a_truncated_file_list_counts_as_protected", () => {
  // `pulls.listFiles` stops at LISTED_FILES_CAP files even when paginated, so
  // beyond it the list is silently short. A `.github/` change past the cut
  // simply isn't in `files`, and answering "no protected paths" would be
  // answering a question the data can't support - exactly for the bulk or
  // generated diff this guard is meant to catch.
  const ordinary = (n) => Array.from({ length: n }, (_, i) => ({ filename: `src/f${i}.rs` }));

  assert.equal(autoMergeBlockedReason(ordinary(LISTED_FILES_CAP - 1)), null);

  const capped = autoMergeBlockedReason(ordinary(LISTED_FILES_CAP));
  assert.match(capped, /file cap/);
  assert.match(capped, /cannot be shown not to change a gate/);
});

test("blocked_reason_names_the_protected_path_case_separately", () => {
  assert.equal(autoMergeBlockedReason([{ filename: "docs/backups.md" }]), null);

  // The reason names the offending file itself, so the log line says which
  // rail the PR touched rather than only that it touched one.
  assert.equal(
    autoMergeBlockedReason([{ filename: ".github/workflows/ci.yml" }]),
    "it changes .github/workflows/ci.yml",
  );
  assert.equal(
    autoMergeBlockedReason([{ filename: "crates/server/src/main.rs" }, { filename: "deny.toml" }]),
    "it changes deny.toml",
  );
  assert.equal(
    autoMergeBlockedReason([{ filename: "frontend/package.json" }]),
    "it changes frontend/package.json",
  );
});

function fakeReviews(reviews) {
  return { paginate: async () => reviews, rest: { pulls: { listReviews: {} } } };
}

test("an_approval_only_counts_for_the_commit_it_was_submitted_against", async () => {
  // The mirror of changesRequestedIsCurrent, and for the same reason: GitHub
  // dismisses an approval on a new commit only when branch protection says
  // to, and this script cannot see that setting. Without this check a review
  // of commit A still reads as APPROVED after an unreviewed commit B.
  const approvedHead = fakeReviews([
    { user: { login: "human" }, state: "APPROVED", commit_id: "head", submitted_at: "2026-01-02" },
  ]);
  assert.equal(await approvalIsCurrent(approvedHead, "o", "r", 7, "head"), true);

  const approvedOlder = fakeReviews([
    { user: { login: "human" }, state: "APPROVED", commit_id: "older", submitted_at: "2026-01-02" },
  ]);
  assert.equal(await approvalIsCurrent(approvedOlder, "o", "r", 7, "head"), false);
});

test("a_later_comment_does_not_retract_a_standing_approval", async () => {
  // A COMMENTED review is a separate review object with its own submitted_at,
  // and leaving one does not retract a verdict - "LGTM, one nit for a
  // follow-up" after approving is an ordinary workflow, and GitHub still
  // reports reviewDecision APPROVED. If COMMENTED could be someone's "latest",
  // it would mask their approval and auto-merge would refuse a genuine,
  // current one: a false negative that breaks the feature rather than merely
  // being conservative.
  const github = fakeReviews([
    { user: { login: "human" }, state: "APPROVED", commit_id: "head", submitted_at: "2026-01-01" },
    { user: { login: "human" }, state: "COMMENTED", commit_id: "head", submitted_at: "2026-01-02" },
  ]);
  assert.equal(await approvalIsCurrent(github, "o", "r", 7, "head"), true);
});

test("a_dismissed_review_is_not_an_approval", async () => {
  // Worth pinning on its own, but note what it does *not* prove: because
  // dismissal mutates a review's state in place, an approval that was
  // dismissed is no longer an APPROVED object at all, so this passes whether
  // or not DISMISSED is in VERDICT_REVIEW_STATES. Excluding it there is
  // tidiness, not the load-bearing part - that is the COMMENTED case above.
  const github = fakeReviews([
    { user: { login: "human" }, state: "DISMISSED", commit_id: "head", submitted_at: "2026-01-01" },
  ]);
  assert.equal(await approvalIsCurrent(github, "o", "r", 7, "head"), false);
});

test("only_a_reviewers_latest_review_counts_towards_currency", async () => {
  // Mirrors how GitHub computes reviewDecision. A reviewer who approved this
  // exact head and then came back asking for changes has not approved it.
  const github = fakeReviews([
    { user: { login: "human" }, state: "APPROVED", commit_id: "head", submitted_at: "2026-01-01" },
    {
      user: { login: "human" },
      state: "CHANGES_REQUESTED",
      commit_id: "head",
      submitted_at: "2026-01-02",
    },
  ]);
  assert.equal(await approvalIsCurrent(github, "o", "r", 7, "head"), false);
});

test("auto_merge_refuses_when_the_approval_does_not_cover_the_head", async () => {
  const { github, calls } = fakeGithub([{ filename: "docs/backups.md" }]);
  const messages = [];

  await autoMergeIfApproved(
    github,
    { info: (m) => messages.push(m) },
    "o",
    "r",
    7,
    samePrRepo,
    false,
  );

  assert.deepEqual(calls.merged, [], "a stale approval must not merge unattended");
  assert.deepEqual(calls.deletedRefs, [], "and the branch must survive");
  assert.match(messages.join("\n"), /no approving review was submitted against this exact commit/);
});

test("auto_merge_refuses_when_a_caller_forgets_to_say_whether_the_approval_is_current", async () => {
  // `!== true` rather than a falsy check with a default: the approval is the
  // one input that cannot be re-derived from the PR here, so a caller that
  // omits it must get a refusal rather than a merge.
  const { github, calls } = fakeGithub([{ filename: "docs/backups.md" }]);

  await autoMergeIfApproved(github, { info: () => {} }, "o", "r", 7, samePrRepo);

  assert.deepEqual(calls.merged, [], "a missing answer must mean no");
});
