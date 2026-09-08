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
  autoMergeIfApproved,
  parseAutoMergeEnabled,
  resolveAutoMerge,
  touchesProtectedPaths,
  AUTO_MERGE_PROTECTED_PREFIX,
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
  // Every gate this automation trusts lives under .github/ - the coverage
  // analyzer, the duplicate-code check, the workflows, and the script that
  // decides what "ready to merge" means. A PR editing any of them must wait
  // for a person, or the rails could widen themselves on the automation's
  // own approval.
  assert.equal(AUTO_MERGE_PROTECTED_PREFIX, ".github/");

  for (const filename of [
    ".github/scripts/analyze-coverage-diff.js",
    ".github/scripts/lib/lcov.js",
    ".github/scripts/sync-pr-labels.js",
    ".github/workflows/ci.yml",
    ".github/workflows/coverage-diff-check.yml",
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
      { filename: "scripts/analyze-coverage-diff.js", previous_filename: ".github/scripts/analyze-coverage-diff.js" },
    ]),
    true,
  );
});

test("auto_merge_guard_is_not_fooled_by_a_lookalike_path", () => {
  assert.equal(touchesProtectedPaths([{ filename: "docs/.github/notes.md" }]), false);
  assert.equal(touchesProtectedPaths([{ filename: "vendor/x.github/thing.yml" }]), false);
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
function fakeGithub(files) {
  const calls = { merged: [], deletedRefs: [] };
  const github = {
    paginate: async () => files,
    rest: {
      pulls: {
        listFiles: {},
        merge: async (args) => {
          calls.merged.push(args);
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
const samePrRepo = { head: { ref: "feature", repo: { id: 1 } }, base: { repo: { id: 1 } } };

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

  await autoMergeIfApproved(github, { info: () => {} }, "o", "r", 7, samePrRepo);

  assert.equal(calls.merged.length, 1, "an ordinary PR still merges");
  assert.deepEqual(calls.merged[0], {
    owner: "o",
    repo: "r",
    pull_number: 7,
    merge_method: "squash",
  });
  assert.equal(calls.deletedRefs.length, 1, "and its same-repo branch is deleted");
  assert.equal(calls.deletedRefs[0].ref, "heads/feature");
});

test("auto_merge_leaves_a_fork_branch_alone", async () => {
  const { github, calls } = fakeGithub([{ filename: "docs/backups.md" }]);
  const forkPr = { head: { ref: "feature", repo: { id: 2 } }, base: { repo: { id: 1 } } };

  await autoMergeIfApproved(github, { info: () => {} }, "o", "r", 7, forkPr);

  assert.equal(calls.merged.length, 1);
  assert.deepEqual(calls.deletedRefs, [], "this token can't delete a fork's branch");
});
