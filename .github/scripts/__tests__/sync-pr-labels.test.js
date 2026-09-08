// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

// Pins the two decisions that stand between "the automation approved it" and
// "the automation merged it": how the AUTO_MERGE_ENABLED kill switch is read,
// and what auto-merge refuses to land on its own.

const assert = require("node:assert/strict");
const { test } = require("node:test");

const {
  parseAutoMergeEnabled,
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
