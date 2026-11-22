// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

// Gate for `npm audit --json` output with a human-maintained allowlist.
//
// Every allowlisted advisory MUST name the tracking issue that will remove it
// ({ "package": "...", "issue": "https://github.com/<owner>/<repo>/issues/<n>" }),
// so an accepted vulnerability is always a ticket someone can pick up and
// close, never a silent suppression. An entry without a valid issue link makes
// the check fail.
//
// Usage: node check-npm-audit.js <audit.json> <allowlist.json>
// Issue links must belong to $GITHUB_REPOSITORY when it is set (it is in CI).

const fs = require("node:fs");

const BLOCKING_SEVERITIES = new Set(["moderate", "high", "critical"]);
const ISSUE_URL = /^https:\/\/github\.com\/[\w.-]+\/[\w.-]+\/issues\/[1-9]\d*$/;

/**
 * Validate the allowlist and return its problems (empty when well-formed).
 * When `repo` ("owner/name") is given, issue links must point into that repo,
 * so the "tracked" guarantee can't be met with a link to somewhere else.
 */
function validateAllowlist(allowlist, repo) {
  const entries = allowlist && allowlist.audit;
  if (!Array.isArray(entries)) return ['allowlist must have an "audit" array'];
  const problems = [];
  const seen = new Set();
  entries.forEach((entry, i) => {
    const where = `audit[${i}]`;
    if (!entry || typeof entry !== "object" || Array.isArray(entry)) {
      problems.push(`${where}: must be an object { "package", "issue" }`);
      return;
    }
    if (typeof entry.package !== "string" || entry.package.trim() === "") {
      problems.push(`${where}: missing "package"`);
    } else if (seen.has(entry.package)) {
      problems.push(`${where}: duplicate entry for "${entry.package}"`);
    } else {
      seen.add(entry.package);
    }
    if (typeof entry.issue !== "string" || !ISSUE_URL.test(entry.issue)) {
      problems.push(
        `${where} (${entry.package}): "issue" must be a GitHub issue URL tracking its removal`,
      );
    } else if (repo) {
      const prefix = `https://github.com/${repo}/issues/`.toLowerCase();
      if (!entry.issue.toLowerCase().startsWith(prefix)) {
        problems.push(`${where} (${entry.package}): "issue" must be an issue in ${repo}`);
      }
    }
  });
  return problems;
}

/**
 * Decide the outcome. Returns { ok, errors, warnings } so callers (and tests)
 * never need to parse console output.
 */
function evaluate(audit, allowlist, repo) {
  const errors = validateAllowlist(allowlist, repo);
  const warnings = [];

  // `npm audit --json` reports its own failures (registry unreachable, ...) as
  // a top-level error object with no vulnerabilities map. Treat that as a
  // failure, never as "nothing found".
  if (!audit || audit.error || typeof audit.vulnerabilities !== "object") {
    const detail = audit && audit.error ? ` (${audit.error.summary || audit.error.code})` : "";
    errors.push(`npm audit did not produce a usable report${detail}`);
    return { ok: false, errors, warnings };
  }

  const allowed = new Set(
    Array.isArray(allowlist && allowlist.audit)
      ? allowlist.audit.filter((e) => e && typeof e.package === "string").map((e) => e.package)
      : [],
  );
  const flagged = Object.entries(audit.vulnerabilities)
    .filter(([, v]) => BLOCKING_SEVERITIES.has(v.severity))
    .map(([name, v]) => ({ name, severity: v.severity }));

  for (const { name, severity } of flagged) {
    if (!allowed.has(name)) errors.push(`unallowed ${severity} vulnerability in "${name}"`);
  }
  const flaggedNames = new Set(flagged.map((f) => f.name));
  for (const name of allowed) {
    if (!flaggedNames.has(name)) {
      warnings.push(`allowlist entry "${name}" is no longer flagged - remove it and close its issue`);
    }
  }
  return { ok: errors.length === 0, errors, warnings };
}

function main(argv) {
  const [auditPath, allowlistPath] = argv;
  if (!auditPath || !allowlistPath) {
    console.error("usage: check-npm-audit.js <audit.json> <allowlist.json>");
    return 2;
  }
  let audit;
  try {
    audit = JSON.parse(fs.readFileSync(auditPath, "utf-8"));
  } catch (e) {
    console.error(`could not read npm audit output: ${e.message}`);
    return 1;
  }
  const allowlist = JSON.parse(fs.readFileSync(allowlistPath, "utf-8"));
  const { ok, errors, warnings } = evaluate(audit, allowlist, process.env.GITHUB_REPOSITORY);
  for (const w of warnings) console.warn(`::warning::${w}`);
  for (const e of errors) console.error(`::error::${e}`);
  if (ok) console.log("npm audit: no unallowed moderate+ vulnerabilities.");
  return ok ? 0 : 1;
}

module.exports = { evaluate, validateAllowlist, ISSUE_URL };

if (require.main === module) process.exit(main(process.argv.slice(2)));
