// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

const test = require("node:test");
const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { join } = require("node:path");

/**
 * The review workflow's prompt and its allowlist have to agree.
 *
 * Every no-verdict review failure this repo has recorded (#351, #399, #425,
 * #441, #469, #513) is the same shape: the prompt sends the reviewer after
 * something the allowlist does not let it reach, the turn budget goes on
 * permission denials, and the run exits cleanly with no verdict. Nothing in
 * CI can see that - the action reports `is_error: false` - so the checks
 * below pin the parts of that agreement a future edit could silently break.
 */
const WORKFLOW = readFileSync(
  join(__dirname, "..", "..", "workflows", "claude-review.yml"),
  "utf-8",
);

function allowedTools() {
  const match = WORKFLOW.match(/--allowedTools "([^"]+)"/);
  assert.ok(match, "claude-review.yml no longer passes --allowedTools");
  return match[1].split(",").map((entry) => entry.trim());
}

test("the diff is written to the one path the prompt reads it from", () => {
  // The step writes it and the prompt names it, in two places that cannot be
  // kept in sync by anything but this check. A rename on one side alone sends
  // the reviewer to a file that does not exist - which degrades to the
  // shell-out path the fallback names, i.e. exactly the failure being fixed.
  const paths = [...WORKFLOW.matchAll(/review-input\/[\w.-]+/g)].map((m) => m[0]);
  assert.ok(paths.length >= 2, "expected the diff path in both the step and the prompt");
  assert.equal(
    new Set(paths).size,
    1,
    `the step and the prompt name different diff paths: ${[...new Set(paths)].join(", ")}`,
  );
});

test("the reviewer can read that file with a tool it actually has", () => {
  // `Read` is what makes writing the diff to disk work at all: it pages a
  // long file natively, with no pipe and no redirect for the permission
  // layer to deny.
  assert.ok(allowedTools().includes("Read"), "Read is what the prompt tells it to use");
});

test("no tool that can write outside the workflow's own control is granted", () => {
  // The diff moved onto disk so the reviewer would not need a write
  // primitive - not so it could be given one. A review only ever reads.
  const granted = allowedTools();
  for (const tool of ["Write", "Edit", "MultiEdit", "NotebookEdit", "WebFetch", "WebSearch"]) {
    assert.ok(!granted.includes(tool), `${tool} must not be in the review allowlist`);
  }
});

test("a failure fetching the diff cannot strand the PR without a verdict comment", () => {
  // The "Sync claude-review-failed signal" step is what labels and comments
  // a review that produced nothing. A hard failure in the diff step would
  // skip it, putting the PR back in the silent `needs review` state that
  // step exists to prevent - so the diff step tolerates its own failure and
  // the prompt carries a fallback.
  const step = WORKFLOW.slice(
    WORKFLOW.indexOf("- name: Materialize the PR diff for the reviewer"),
    WORKFLOW.indexOf("- name: Run Claude review"),
  );
  assert.ok(step.length > 0, "the diff step must run before the review step");
  assert.match(step, /continue-on-error: true/);
  assert.match(WORKFLOW, /If `review-input\/pr\.diff` is missing/);
});
