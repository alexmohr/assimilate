// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

// Minimal LCOV reader - only what analyze-coverage-diff.js needs (per-file
// line hit counts and aggregate line totals), not a general-purpose parser.

const path = require("path");

// Maps an lcov `SF:` path to the repository-relative path GitHub reports for
// a PR's changed files, or null when the source isn't part of the repository.
//
// cargo-llvm-cov and the e2e backend report both write absolute paths under
// the CI workspace, so without this the per-line check looked every Rust file
// up under `crates/...` and never found it: new Rust lines went unchecked.
// (The frontend reports are already repository-relative - ci.yml rewrites
// them to `frontend/src/...` before merging.)
//
// Sources outside the workspace (the toolchain's own `/rustc/<hash>/library`
// files, which instrument whichever dependency thread-locals a given run
// happened to initialise) are dropped: they aren't this repository's code, so
// no change here can cover or uncover them.
function toRepoPath(sourcePath, workspaceRoot) {
  const normalized = path.posix.normalize(sourcePath);
  if (path.posix.isAbsolute(normalized)) {
    if (!workspaceRoot) return null;
    const root = `${path.posix.normalize(workspaceRoot).replace(/\/$/, "")}/`;
    return normalized.startsWith(root) ? normalized.slice(root.length) : null;
  }
  return normalized;
}

// Returns `{ files, droppedLines }`, `files` being path -> Map(line -> hits).
// Records for the same file (after `toRepoPath`) are summed line by line - a
// line any suite executed counts as covered, the same way
// `lcov --add-tracefile` merges the unit and e2e reports.
//
// `droppedLines` counts the instrumented lines whose source lies outside the
// repository, so a caller can tell "nothing outside the workspace" apart from
// "the workspace root was wrong and everything was thrown away".
function parseLcov(content, { workspaceRoot } = {}) {
  const files = new Map();
  let current = null;
  let skipping = false;
  let droppedLines = 0;

  for (const rawLine of content.split("\n")) {
    const line = rawLine.trim();
    if (line.startsWith("SF:")) {
      const repoPath = toRepoPath(line.slice(3), workspaceRoot);
      skipping = repoPath === null;
      current = skipping ? null : files.get(repoPath) || new Map();
      if (current) files.set(repoPath, current);
    } else if (line.startsWith("DA:")) {
      if (skipping) {
        droppedLines += 1;
        continue;
      }
      if (!current) continue;
      const [lineNoStr, hitsStr] = line.slice(3).split(",");
      const lineNo = Number(lineNoStr);
      const hits = Number(hitsStr);
      current.set(lineNo, (current.get(lineNo) || 0) + hits);
    } else if (line === "end_of_record") {
      current = null;
      skipping = false;
    }
  }

  return { files, droppedLines };
}

function totals(files) {
  let coveredLines = 0;
  let totalLines = 0;
  for (const lineHits of files.values()) {
    for (const hits of lineHits.values()) {
      totalLines += 1;
      if (hits > 0) coveredLines += 1;
    }
  }
  return {
    totalLines,
    coveredLines,
    percent: totalLines === 0 ? 100 : (coveredLines / totalLines) * 100,
  };
}

// Lines instrumented in both reports that `base` executed and `pr` did not,
// restricted to files `isComparable` accepts. What turns an aggregate drop
// into something actionable: either a test that used to reach these lines was
// removed or weakened, or reaching them depends on timing and needs a
// deterministic test (see #353).
function lostLines(base, pr, isComparable) {
  const lost = [];
  for (const [file, baseHits] of base) {
    if (!isComparable(file)) continue;
    const prHits = pr.get(file);
    if (!prHits) continue;
    for (const [lineNo, hits] of baseHits) {
      if (hits > 0 && prHits.get(lineNo) === 0) lost.push({ file, lineNo });
    }
  }
  return lost.sort((a, b) => a.file.localeCompare(b.file) || a.lineNo - b.lineNo);
}

module.exports = { parseLcov, totals, toRepoPath, lostLines };
