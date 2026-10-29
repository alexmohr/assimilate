// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

// Deterministic "new/changed code must be covered" + "coverage must not
// regress" checks (no line present on both sides loses coverage, no more
// lines go uncovered), so the review workflow doesn't have to spend a Claude
// turn re-deriving facts an lcov report already answers.
//
// Runs solely in coverage-diff-check.yml, which owns the PR comment, the
// `coverage failed` label, AND a "Coverage Diff Check" GitHub check run on
// the commit. claude-review.yml never runs this analysis itself - it
// can't, since it triggers on the same "CI completed" event this workflow
// does with no ordering guarantee, so re-running the check inline would
// either race coverage-diff-check.yml's own writes (the bug that used to
// duplicate the PR comment) or still be gating on a guess. Instead
// pre-review-checks.js waits for every check run on the commit
// (lib/wait-for-check.js, not specific to this one by name) to reach an
// authoritative, already-finished conclusion. See skills/review/SKILL.md
// for the full reasoning.

const fs = require("fs");
const { parseLcov, totals, lostLines } = require("./lib/lcov");
const syncLabels = require("./sync-pr-labels");
const { upsertMarkedComment } = require("./lib/pr-comment");

const MARKER = "<!-- coverage-diff-check -->";
const CHECK_NAME = "Coverage Diff Check";

// Files where "this line has no coverage" isn't a meaningful finding: the
// tests themselves, generated code, and e2e specs (exercised through the
// browser, not instrumented the same way as unit-tested source).
const EXCLUDED_PATHS = [
  /^frontend\/src\/types\/generated\//,
  /(^|\/)tests?\//i,
  /\.(test|spec)\.[jt]sx?$/,
  /^frontend\/e2e\//,
  /_test\.rs$/,
];

function isExcluded(file) {
  return EXCLUDED_PATHS.some((p) => p.test(file));
}

// How many lost lines a regression finding spells out: enough to see whether
// they cluster in one module.
const MAX_LOST_LINES_LISTED = 20;

function listLines(lines) {
  const listed = lines
    .slice(0, MAX_LOST_LINES_LISTED)
    .map(({ file, lineNo }) => `\`${file}:${lineNo}\``)
    .join(", ");
  const more = lines.length > MAX_LOST_LINES_LISTED ? ` and ${lines.length - MAX_LOST_LINES_LISTED} more` : "";
  return `${listed}${more}`;
}

function describeRegression({ lostOutsideDiff, lostInDiff, uncoveredDelta }) {
  let text = "";
  if (lostOutsideDiff.length > 0) {
    text +=
      ` ${lostOutsideDiff.length} line(s) in files this PR does not change lost coverage: ` +
      `${listLines(lostOutsideDiff)}. If no test that reached them was removed or weakened, reaching ` +
      "them depends on timing or test order - give them a deterministic test rather than re-running until it passes.";
  }
  if (lostInDiff.length > 0) {
    if (lostOutsideDiff.length === 0) {
      text += " No line in a file this PR leaves alone lost coverage, so the regression comes from the files it changes.";
    }
    text += ` ${lostInDiff.length} line(s) this PR keeps in files it changes lost coverage: ${listLines(lostInDiff)}.`;
  }
  if (uncoveredDelta > 0) {
    text += ` This PR has ${uncoveredDelta} more uncovered line(s) than main.`;
  }
  return text;
}

function readReport(filePath, workspaceRoot) {
  const { files, droppedLines } = parseLcov(fs.readFileSync(filePath, "utf8"), { workspaceRoot });
  return { files, droppedLines, totals: totals(files) };
}

// Walks a unified diff's hunks: `onHunk(oldLine, newLine)` at each hunk's
// first line, `onLine(oldLine, newLine)` for every line in it, with null on
// the side a line doesn't exist on (old for an added line, new for a removed
// one). The "no newline at end of file" marker ('\') is neither.
function walkPatch(patch, { onHunk = () => {}, onLine }) {
  let oldLine = 0;
  let newLine = 0;
  let inHunk = false;
  for (const line of patch.split("\n")) {
    const hunk = line.match(/^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/);
    if (hunk) {
      // An empty side's start names the line *before* the hunk, not its first.
      oldLine = Number(hunk[1]) + (hunk[2] === "0" ? 1 : 0);
      newLine = Number(hunk[3]) + (hunk[4] === "0" ? 1 : 0);
      inHunk = true;
      onHunk(oldLine, newLine);
      continue;
    }
    if (!inHunk || line === "" || line.startsWith("\\")) continue;
    const added = line.startsWith("+");
    const removed = line.startsWith("-");
    onLine(added ? null : oldLine, removed ? null : newLine);
    if (!added) oldLine += 1;
    if (!removed) newLine += 1;
  }
}

function addedLineNumbers(patch) {
  const added = [];
  if (!patch) return added;
  walkPatch(patch, {
    onLine: (oldLine, newLine) => {
      if (oldLine === null) added.push(newLine);
    },
  });
  return added;
}

// Maps each line number of a file's old version to its number in the new
// version, or to null when the patch deletes it. Lines outside every hunk
// shift by however many lines the hunks before them added or removed.
function oldToNewLine(patch) {
  const hunks = [];
  let current = null;
  walkPatch(patch, {
    onHunk: (oldLine, newLine) => {
      current = { oldStart: oldLine, oldEnd: oldLine, newEnd: newLine, mapped: new Map() };
      hunks.push(current);
    },
    onLine: (oldLine, newLine) => {
      if (oldLine !== null) {
        current.mapped.set(oldLine, newLine);
        current.oldEnd = oldLine + 1;
      }
      if (newLine !== null) current.newEnd = newLine + 1;
    },
  });
  return (lineNo) => {
    let delta = 0;
    for (const { oldStart, oldEnd, newEnd, mapped } of hunks) {
      if (lineNo < oldStart) break;
      if (lineNo < oldEnd) return mapped.get(lineNo) ?? null;
      delta = newEnd - oldEnd;
    }
    return lineNo + delta;
  };
}

// Where a line of main's report lives in the PR's: unchanged files line up
// one to one, changed files through their patch. A deleted file's patch maps
// every line to null, so it contributes no lost lines; a changed file without
// a patch (GitHub omits it for binary or very large diffs) can't be lined up
// and contributes none either - the uncovered-line count still catches a
// regression there.
function counterpartIn(files) {
  const changes = new Map();
  for (const file of files) {
    changes.set(file.previous_filename || file.filename, {
      file: file.filename,
      mapLine: file.patch ? oldToNewLine(file.patch) : null,
    });
  }
  return (path, lineNo) => {
    const change = changes.get(path);
    if (!change) return { file: path, lineNo };
    const mapped = change.mapLine?.(lineNo) ?? null;
    return mapped === null ? null : { file: change.file, lineNo: mapped };
  };
}

// Pure report analysis, kept separate from the GitHub orchestration below so
// it's independently reusable/testable (also used directly by
// pre-review-checks.js's predecessor logic - see git history).
//
// `workspaceRoot` is the directory CI checked the repository out into; the
// Rust reports' absolute paths are made relative to it (see lib/lcov.js).
async function analyzeDiff({ github, owner, repo, prNumber, prLcovPath, baseLcovPath, workspaceRoot }) {
  const findings = [];

  // Either artifact can be legitimately missing: the PR's own CI run may
  // predate the coverage-final upload (an already-open PR that hasn't been
  // pushed to since), or main may not have a run with it yet. Don't block
  // the review pipeline on historical data that can't exist yet - the check
  // becomes active again once a fresh push produces both artifacts.
  if (!fs.existsSync(prLcovPath) || !fs.existsSync(baseLcovPath)) {
    return { ok: true, findings: [] };
  }

  const pr = readReport(prLcovPath, workspaceRoot);
  const base = readReport(baseLcovPath, workspaceRoot);

  // Sources outside the workspace are dropped as not this repository's code.
  // When that is most of a report the workspace root is wrong, and comparing
  // what is left would quietly stop gating every Rust file - so fail loudly.
  const unmapped = [
    ["this PR", pr],
    ["main", base],
  ].filter(([, report]) => report.droppedLines > report.totals.totalLines);
  if (unmapped.length > 0) {
    return {
      ok: false,
      findings: unmapped.map(
        ([side, report]) =>
          `Most of ${side}'s coverage report (${report.droppedLines} of ` +
          `${report.droppedLines + report.totals.totalLines} lines) lies outside the workspace ` +
          `\`${workspaceRoot}\`, so its paths can't be matched to this repository's files.`,
      ),
    };
  }

  const files = await github.paginate(github.rest.pulls.listFiles, {
    owner,
    repo,
    pull_number: prNumber,
    per_page: 100,
  });

  // Raw percentages are not compared: with coverage below 100%, deleting a
  // covered line always lowers the percentage, so a PR removing tested dead
  // code would fail although no test lost any reach. What does regress is a
  // line present on both sides that a test reached on main and none reaches
  // now, or more lines no test reaches at all. Both are zero tolerance.
  // Tests, e2e specs and generated code are left out of both, as they are of
  // the new-line check below: a removed test shows up as lost coverage in the
  // source it exercised, not in its own helpers.
  const changedPaths = new Set(files.map((file) => file.filename));
  const lost = lostLines(base.files, pr.files, counterpartIn(files)).filter(({ file }) => !isExcluded(file));
  // Counted over the whole report, not just the PR's changed files, so a file
  // GitHub sends no patch for is still covered; it skips the same paths the
  // new-line check below does, so adding generated code or test helpers no
  // test executes isn't a regression.
  const uncovered = (report) => {
    let count = 0;
    for (const [file, lineHits] of report.files) {
      if (isExcluded(file)) continue;
      for (const hits of lineHits.values()) if (hits === 0) count += 1;
    }
    return count;
  };
  const regression = {
    lostOutsideDiff: lost.filter(({ file }) => !changedPaths.has(file)),
    lostInDiff: lost.filter(({ file }) => changedPaths.has(file)),
    uncoveredDelta: uncovered(pr) - uncovered(base),
  };
  if (lost.length > 0 || regression.uncoveredDelta > 0) {
    const from = `${base.totals.percent.toFixed(2)}% (main)`;
    const to = `${pr.totals.percent.toFixed(2)}% (this PR)`;
    const headline =
      pr.totals.percent < base.totals.percent
        ? `Aggregate line coverage decreased from ${from} to ${to}`
        : `Line coverage regressed although aggregate coverage went from ${from} to ${to}`;
    findings.push(
      `${headline} - check for removed or weakened tests, even if no specific uncovered line is flagged below.` +
        describeRegression(regression),
    );
  }

  for (const file of files) {
    if (isExcluded(file.filename)) continue;
    const lineHits = pr.files.get(file.filename);
    if (!lineHits) continue; // not an instrumented file (docs, config, ...)

    for (const lineNo of addedLineNumbers(file.patch)) {
      if (!lineHits.has(lineNo)) continue; // not an executable line
      if (lineHits.get(lineNo) === 0) {
        findings.push(`${file.filename}:${lineNo} is new/changed but has no test coverage.`);
      }
    }
  }

  return { ok: findings.length === 0, findings };
}

// `baselineNote` says which `main` commit the PR was compared against (see
// lib/coverage-baseline.js), so a reader can tell an exact comparison from
// the latest-`main` fallback.
async function publishCheckRun(github, owner, repo, headSha, ok, findings, baselineNote) {
  const verdict = ok
    ? "No new/changed lines are uncovered, no line lost coverage, and no uncovered lines were added."
    : findings.join("\n");
  await github.rest.checks.create({
    owner,
    repo,
    name: CHECK_NAME,
    head_sha: headSha,
    status: "completed",
    conclusion: ok ? "success" : "failure",
    output: {
      title: ok ? "Coverage-diff check passed" : "Coverage-diff check failed",
      summary: baselineNote ? `${verdict}\n\n${baselineNote}` : verdict,
    },
  });
}

module.exports = async ({
  github,
  context,
  core,
  prNumber,
  headSha,
  prLcovPath,
  baseLcovPath,
  workspaceRoot,
  baselineNote,
}) => {
  const owner = context.repo.owner;
  const repo = context.repo.repo;

  const { ok, findings } = await analyzeDiff({
    github,
    owner,
    repo,
    prNumber,
    prLcovPath,
    baseLcovPath,
    workspaceRoot,
  });

  // Published first and unconditionally: this is the signal
  // pre-review-checks.js polls for via lib/wait-for-check.js, so it must
  // land regardless of what the comment/label steps below do.
  await publishCheckRun(github, owner, repo, headSha, ok, findings, baselineNote);

  if (findings.length === 0) {
    // Nothing to say - don't spam an "all good" comment on every clean run.
    // If a prior failing comment exists, replace it so it doesn't read stale.
    await upsertMarkedComment(github, owner, repo, prNumber, MARKER, `${MARKER}\nCoverage-diff check passed.`, {
      onlyIfExists: true,
    });
  } else {
    const body =
      `${MARKER}\n**Coverage-diff check failed** - this is a deterministic finding ` +
      "(not from Claude); fix it before a review is worth spending on:\n\n" +
      findings.map((f) => `- ${f}`).join("\n") +
      (baselineNote ? `\n\n${baselineNote}` : "");
    await upsertMarkedComment(github, owner, repo, prNumber, MARKER, body);
  }

  const label = syncLabels.COVERAGE_LABEL;
  if (!ok) {
    await syncLabels.ensureLabelExists(github, owner, repo, label);
    await github.rest.issues.addLabels({ owner, repo, issue_number: prNumber, labels: [label.name] });
    core.info(`PR #${prNumber}: coverage-diff check failed (${findings.length} finding(s)).`);
  } else {
    // This workflow owns the label's full lifecycle (see COVERAGE_LABEL in
    // sync-pr-labels.js) - explicitly clear it here rather than relying on a
    // generic synchronize-triggered clear, since coverage-diff-check.yml
    // reacts to the same "workflow_run: CI completed" event
    // pr-status-labels.yml does, and a blind clear there could otherwise
    // race a fresh failing result from this workflow.
    await github.rest.issues
      .removeLabel({ owner, repo, issue_number: prNumber, name: label.name })
      .catch((err) => {
        if (err.status !== 404) throw err;
      });
    core.info(`PR #${prNumber}: coverage-diff check passed.`);
  }

  // Deliberately NOT calling the full syncLabels() here: that call
  // republished the shared "PR Merge Gate" check from within this workflow's
  // own check suite, which is why it showed up confusingly grouped as
  // "Coverage Diff Check / PR Merge Gate" - a snapshot from a single stage,
  // not the actual gate.
  //
  // A re-sync does have to happen though, and this used to claim it didn't:
  // "whichever trigger runs it next" is not a real trigger. sync-pr-labels.js
  // waits for a completed "Coverage Diff Check" run on the head commit before
  // it will leave `pending`, and the check only exists once publishCheckRun
  // above has returned - seconds after the sync job triggered by the same
  // "CI completed" event already read the commit's checks and gave up. With
  // no later event on that commit, the gate sat at in_progress ("still
  // waiting on: Coverage Diff Check") permanently. coverage-diff-check.yml's
  // "Notify the merge gate" step now fires a `precheck-complete`
  // repository_dispatch once this function returns, which re-runs the sync
  // from pr-status-labels.yml's own workflow (and so its own check suite),
  // keeping the grouping problem above solved.

  return { ok, findings };
};

module.exports.analyzeDiff = analyzeDiff;
