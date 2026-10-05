// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

const test = require("node:test");
const assert = require("node:assert/strict");
const { execFileSync } = require("node:child_process");
const {
  chmodSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} = require("node:fs");
const { tmpdir } = require("node:os");
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

/**
 * Runs the diff step's own script with `gh` stubbed out, in a scratch
 * directory, and hands the assertions that directory.
 *
 * Running the script is the point: a check that the YAML contains an `mv`
 * would pass on a script that never reaches it. The extract is asserted to
 * still contain the command for the same reason - an extraction bug would
 * otherwise turn these into tests that pass by doing nothing.
 */
function withDiffStep(ghStub, assertions) {
  const script = stepScript("Materialize the PR diff for the reviewer");
  assert.match(
    script,
    /gh pr diff/,
    "the extracted script is not the diff step",
  );

  const dir = mkdtempSync(join(tmpdir(), "review-diff-"));
  try {
    const bin = join(dir, "bin");
    mkdirSync(bin);
    writeFileSync(join(bin, "gh"), ghStub);
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

    assertions(dir);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

test("a gh failure leaves no diff file behind for the reviewer to trust", () => {
  // The failure this guards is quiet: bash truncates a redirect target before
  // the command runs, so `gh > pr.diff` that dies part way leaves an empty or
  // partial file. Present-but-empty does not match the prompt's "is missing"
  // fallback, so the reviewer would read "no changes" and review a PR it
  // never saw - a wrong verdict wearing the shape of a normal one.
  const stubs = [
    // Dies part way through, after writing some of the diff.
    '#!/bin/sh\nprintf "diff --git a/x b/x\\n--- a/x\\n"\nexit 1\n',
    // Dies immediately, writing nothing.
    "#!/bin/sh\nexit 1\n",
  ];

  for (const stub of stubs) {
    withDiffStep(stub, (dir) => {
      assert.equal(
        existsSync(join(dir, "review-input", "pr.diff")),
        false,
        "a failed gh must leave no pr.diff, so the prompt's is-missing fallback fires",
      );
    });
  }
});

test("a successful fetch still puts the diff where the prompt looks", () => {
  withDiffStep('#!/bin/sh\nprintf "diff --git a/x b/x\\n"\n', (dir) => {
    assert.match(
      readFileSync(join(dir, "review-input", "pr.diff"), "utf-8"),
      /^diff --git/,
    );
    // The scratch file must not survive: the reviewer is told to read one
    // path, and a leftover sibling is one more thing to explain.
    assert.equal(existsSync(join(dir, "review-input", "pr.diff.part")), false);
  });
});

test("the prompt's fallback command is one the allowlist actually permits", () => {
  // The fallback runs only when the materialize step failed - i.e. on exactly
  // the flaky, large-diff runs most likely to need it, and least likely to be
  // noticed failing, since the action reports `is_error: false` either way.
  // It is also a pipe, the shape that caused the original incidents: allowed
  // only where BOTH sides are listed. Nothing else checks that, so trimming
  // `head` from the allowlist later would silently restore the no-verdict bug
  // this whole file exists to prevent.
  const prompt = WORKFLOW.slice(WORKFLOW.indexOf("- name: Run Claude review"));
  const fallback = prompt.match(/gh pr diff [^\n`]*\| (\w+) -\d+/);
  assert.ok(fallback, "the prompt no longer names a fallback command");

  const granted = allowedTools();
  assert.ok(
    granted.includes("Bash(gh pr diff:*)"),
    "the fallback's left-hand side must be allowlisted",
  );
  assert.ok(
    granted.includes(`Bash(${fallback[1]}:*)`),
    `the fallback pipes into \`${fallback[1]}\`, which is not in the allowlist`,
  );
});

/**
 * The workflow's jobs, keyed by job id, each as the raw text of its block.
 *
 * Text, not parsed YAML: this directory has no YAML dependency and these
 * checks only need to know which job a line belongs to. Jobs sit at exactly
 * two spaces of indentation under `jobs:`, which is all the split relies on.
 */
function jobs() {
  const body = WORKFLOW.slice(WORKFLOW.indexOf("\njobs:\n") + "\njobs:\n".length);
  const result = {};
  let current = null;
  for (const line of body.split("\n")) {
    const header = line.match(/^ {2}([\w-]+):\s*$/);
    if (header) {
      current = header[1];
      result[current] = "";
      continue;
    }
    if (current) result[current] += `${line}\n`;
  }
  return result;
}

/** The text of a job's own `permissions:` block, or null when it has none. */
function jobPermissions(job) {
  const match = job.match(/^ {4}permissions:\n((?: {6}.*\n| *#.*\n)+)/m);
  return match ? match[1] : null;
}

function jobName(job) {
  const match = job.match(/^ {4}name: (.+)$/m);
  assert.ok(match, "every job has a name");
  return match[1].trim();
}

test("only the job that merges is granted contents:write (#506)", () => {
  // The review job runs the reviewer over a fork PR's diff, title and
  // description with GITHUB_TOKEN in its environment. A token there that
  // can write contents is one prompt injection from a pushed branch, tag or
  // merge - so the workflow default stays read and the grant lives on the
  // one job that processes no untrusted content.
  const topLevel = WORKFLOW.slice(0, WORKFLOW.indexOf("\njobs:\n"));
  assert.match(topLevel, /^ {2}contents: read$/m, "workflow-level contents must stay read");
  assert.doesNotMatch(topLevel, /^ {2}contents: write$/m);

  const all = jobs();
  assert.ok(all.review && all.resync, "expected both the review and resync jobs");
  for (const [id, job] of Object.entries(all)) {
    const permissions = jobPermissions(job) ?? "";
    const grantsWrite = /^ {6}contents: write$/m.test(permissions);
    assert.equal(grantsWrite, id === "resync", `job ${id} contents:write grant`);
  }
});

test("the merging job runs no reviewer and loads only trusted scripts", () => {
  const job = jobs().resync;
  assert.doesNotMatch(job, /claude-code-action/, "no reviewer may run under the write token");
  assert.doesNotMatch(job, /review-input/, "no PR diff belongs in the merging job");
  const checkouts = [...job.matchAll(/uses: actions\/checkout@\S+\n((?: {8,}.*\n)+)/g)];
  assert.equal(checkouts.length, 1, "expected exactly one checkout in the merging job");
  assert.match(checkouts[0][1], /ref: \$\{\{ github\.event\.repository\.default_branch \}\}/);
  assert.match(job, /require\(`\$\{process\.env\.GITHUB_WORKSPACE\}\/\.github\/scripts\/sync-pr-labels\.js`\)/);
});

test("the merging job runs exactly when the post-review step it replaced did", () => {
  // That step ran only when the review actually executed, and - its `if:`
  // carrying no status function - only when every earlier step succeeded,
  // which `needs: review` reproduces. The review step itself is
  // continue-on-error, so a failed review still re-syncs.
  const { review, resync } = jobs();
  assert.match(resync, /^ {4}needs: review$/m);
  assert.match(resync, /^ {4}if: needs\.review\.outputs\.run_claude == 'true'$/m);
  assert.doesNotMatch(resync, /always\(\)|failure\(\)|cancelled\(\)/);
  assert.match(review, /^ {6}pr_number: \$\{\{ steps\.resolve\.outputs\.pr_number \}\}$/m);
  assert.match(review, /^ {6}run_claude: \$\{\{ steps\.precheck\.outputs\.run_claude \}\}$/m);
  assert.match(resync, /needs\.review\.outputs\.pr_number/);
});

test("no checkout leaves a token behind in .git/config", () => {
  const checkouts = [...WORKFLOW.matchAll(/uses: actions\/checkout@\S+\n((?: {8,}.*\n)+)/g)];
  assert.ok(checkouts.length >= 3, "expected the review, restore and resync checkouts");
  for (const [, inputs] of checkouts) {
    assert.match(inputs, /persist-credentials: false/);
  }
});

test("every job of this workflow is excluded from its own completeness checks", () => {
  // Both sync calls look at every check run on the PR's head and treat an
  // unfinished one as "not ready". A job of this same workflow missing from
  // either list would be waited on by the run it belongs to.
  const names = Object.values(jobs()).map(jobName);
  const { SELF_CHECK_NAMES } = require("../pre-review-checks.js");
  const resyncList = jobs().resync.match(/selfCheckNames: (\[[^\]]*\])/);
  assert.ok(resyncList, "the resync job no longer passes selfCheckNames");
  const resyncNames = JSON.parse(resyncList[1]);
  for (const name of names) {
    assert.ok(SELF_CHECK_NAMES.includes(name), `pre-review-checks.js does not exclude "${name}"`);
    assert.ok(resyncNames.includes(name), `the resync job does not exclude "${name}"`);
  }
});
