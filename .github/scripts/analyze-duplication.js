// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

// Reads a jscpd JSON report and keeps only clusters that touch this PR's
// changed files - repo-wide duplication unrelated to the diff isn't this
// PR's problem to fix. Hard gate: "Logic is never to be duplicated instead
// of reused" (skills/review/SKILL.md) is a deterministic fact once jscpd has
// flagged a cluster, not something Claude needs to re-derive.

const fs = require("fs");

const normalize = (p) => p.replace(/^\.\//, "");

module.exports = async ({ reportPath, changedFiles }) => {
  if (!fs.existsSync(reportPath)) {
    return { ok: true, findings: [] };
  }

  const report = JSON.parse(fs.readFileSync(reportPath, "utf8"));
  const duplicates = report.duplicates || [];
  const changedSet = new Set(changedFiles.map(normalize));
  const findings = [];

  for (const dup of duplicates) {
    const first = dup.firstFile;
    const second = dup.secondFile;
    if (!first || !second) continue;

    const firstName = normalize(first.name);
    const secondName = normalize(second.name);
    if (!changedSet.has(firstName) && !changedSet.has(secondName)) continue;

    findings.push(
      `Duplicate code (${dup.lines} lines, ${dup.tokens} tokens): ` +
        `${firstName}:${first.start}-${first.end} matches ${secondName}:${second.start}-${second.end}.`,
    );
  }

  return { ok: findings.length === 0, findings };
};
