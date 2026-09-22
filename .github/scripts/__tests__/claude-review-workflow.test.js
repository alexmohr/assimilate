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
  // `.part` is the step's own scratch file - the same destination mid-write,
  // not a second one the prompt could be pointed at - so it is not a
  // disagreement between the two sides this check exists to keep in sync.
  const paths = [...WORKFLOW.matchAll(/review-input\/[\w.-]+/g)]
    .map((m) => m[0])
    .filter((path) => !path.endsWith(".part"));
  assert.ok(
    paths.length >= 2,
    "expected the diff path in both the step and the prompt",
  );
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
  assert.ok(
    allowedTools().includes("Read"),
    "Read is what the prompt tells it to use",
  );
});

test("no tool that can write outside the workflow's own control is granted", () => {
  // The diff moved onto disk so the reviewer would not need a write
  // primitive - not so it could be given one. A review only ever reads.
  const granted = allowedTools();
  for (const tool of [
    "Write",
    "Edit",
    "MultiEdit",
    "NotebookEdit",
    "WebFetch",
    "WebSearch",
  ]) {
    assert.ok(
      !granted.includes(tool),
      `${tool} must not be in the review allowlist`,
    );
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

/**
 * A step's shell script, dedented so it can be run as-is.
 *
 * Read by indentation rather than matched with a regex: the script contains
 * `#` comment lines, and any pattern that treats a comment as the end of the
 * block silently returns a truncated script - which would then "pass" the
 * checks below by never reaching the code they are about. Asserting the
 * extract still contains the command is the guard against that.
 */
function stepScript(stepName) {
  const lines = WORKFLOW.split("\n");
  const start = lines.findIndex((line) => line.includes(`- name: ${stepName}`));
  assert.ok(start >= 0, `no step named ${stepName}`);

  let runAt = -1;
  for (let i = start + 1; i < lines.length; i++) {
    if (/^\s+- name: /.test(lines[i])) break;
    if (/^\s+run: \|\s*$/.test(lines[i])) {
      runAt = i;
      break;
    }
  }
  assert.ok(runAt > start, `step ${stepName} has no literal run block`);

  const keyIndent = lines[runAt].search(/\S/);
  const body = [];
  for (let i = runAt + 1; i < lines.length; i++) {
    if (lines[i].trim() === "") {
      body.push("");
      continue;
    }
    if (lines[i].search(/\S/) <= keyIndent) break;
    body.push(lines[i]);
  }

  const indent = Math.min(
    ...body.filter((l) => l !== "").map((l) => l.search(/\S/)),
  );
  return body.map((line) => (line === "" ? "" : line.slice(indent))).join("\n");
}

test("a gh failure leaves no diff file behind for the reviewer to trust", () => {
  // The failure this guards is quiet: bash truncates a redirect target before
  // the command runs, so `gh > pr.diff` that dies part way leaves an empty or
  // partial file. Present-but-empty does not match the prompt's "is missing"
  // fallback, so the reviewer would read "no changes" and review a PR it
  // never saw - a wrong verdict wearing the shape of a normal one.
  const { execFileSync } = require("node:child_process");
  const {
    mkdtempSync,
    writeFileSync,
    chmodSync,
    existsSync,
    rmSync,
  } = require("node:fs");
  const { tmpdir } = require("node:os");

  const script = stepScript("Materialize the PR diff for the reviewer");
  assert.match(
    script,
    /gh pr diff/,
    "the extracted script is not the diff step",
  );

  for (const stub of [
    // Dies part way through, after writing some of the diff.
    '#!/bin/sh\nprintf "diff --git a/x b/x\\n--- a/x\\n"\nexit 1\n',
    // Dies immediately, writing nothing.
    "#!/bin/sh\nexit 1\n",
  ]) {
    const dir = mkdtempSync(join(tmpdir(), "review-diff-"));
    try {
      const bin = join(dir, "bin");
      require("node:fs").mkdirSync(bin);
      writeFileSync(join(bin, "gh"), stub);
      chmodSync(join(bin, "gh"), 0o755);

      execFileSync("bash", ["-eo", "pipefail", "-c", script], {
        cwd: dir,
        env: {
          ...process.env,
          PATH: `${bin}:${process.env.PATH}`,
          PR_NUMBER: "1",
        },
        stdio: "ignore",
      });

      assert.equal(
        existsSync(join(dir, "review-input", "pr.diff")),
        false,
        "a failed gh must leave no pr.diff, so the prompt's is-missing fallback fires",
      );
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  }
});

test("a successful fetch still puts the diff where the prompt looks", () => {
  const { execFileSync } = require("node:child_process");
  const {
    mkdtempSync,
    writeFileSync,
    chmodSync,
    readFileSync: read,
    rmSync,
  } = require("node:fs");
  const { tmpdir } = require("node:os");

  const script = stepScript("Materialize the PR diff for the reviewer");
  const dir = mkdtempSync(join(tmpdir(), "review-diff-ok-"));
  try {
    const bin = join(dir, "bin");
    require("node:fs").mkdirSync(bin);
    writeFileSync(
      join(bin, "gh"),
      '#!/bin/sh\nprintf "diff --git a/x b/x\\n"\n',
    );
    chmodSync(join(bin, "gh"), 0o755);

    execFileSync("bash", ["-eo", "pipefail", "-c", script], {
      cwd: dir,
      env: {
        ...process.env,
        PATH: `${bin}:${process.env.PATH}`,
        PR_NUMBER: "1",
      },
      stdio: "ignore",
    });

    assert.match(
      read(join(dir, "review-input", "pr.diff"), "utf-8"),
      /^diff --git/,
    );
    // The temp file must not survive: the reviewer is told to read one path,
    // and a leftover sibling is one more thing to explain.
    assert.equal(
      require("node:fs").existsSync(join(dir, "review-input", "pr.diff.part")),
      false,
    );
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
