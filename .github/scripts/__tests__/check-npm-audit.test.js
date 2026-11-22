// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

const assert = require("node:assert/strict");
const { test } = require("node:test");
const fs = require("node:fs");
const path = require("node:path");

const { evaluate, validateAllowlist } = require("../check-npm-audit.js");

const ISSUE = "https://github.com/alexmohr/assimilate/issues/1";
const clean = { vulnerabilities: {} };
const vuln = (name, severity) => ({ vulnerabilities: { [name]: { severity } } });
const entry = (pkg, issue = ISSUE) => ({ package: pkg, issue });

test("a clean audit with an empty allowlist passes", () => {
  const r = evaluate(clean, { audit: [] });
  assert.equal(r.ok, true);
  assert.deepEqual(r.errors, []);
});

test("an unallowed moderate, high or critical vulnerability fails", () => {
  for (const severity of ["moderate", "high", "critical"]) {
    const r = evaluate(vuln("pug", severity), { audit: [] });
    assert.equal(r.ok, false, severity);
    assert.match(r.errors[0], new RegExp(`${severity}.*"pug"`));
  }
});

test("low severity advisories do not fail the gate", () => {
  assert.equal(evaluate(vuln("pug", "low"), { audit: [] }).ok, true);
});

test("an allowlisted vulnerability with a tracking issue passes", () => {
  const r = evaluate(vuln("pug", "high"), { audit: [entry("pug")] });
  assert.equal(r.ok, true);
});

test("allowing one package does not allow another", () => {
  const r = evaluate(vuln("colors", "high"), { audit: [entry("pug")] });
  assert.equal(r.ok, false);
  assert.ok(r.errors.some((e) => e.includes('"colors"')));
});

test("an allowlist entry without an issue is rejected even if it matches", () => {
  const r = evaluate(vuln("pug", "high"), { audit: [{ package: "pug" }] });
  assert.equal(r.ok, false);
  assert.ok(r.errors.some((e) => e.includes("pug") && e.includes("issue")));
});

test("an issue that is not a GitHub issue URL is rejected", () => {
  for (const issue of ["", "TODO", "#12", "12", "https://example.com/issues/1",
    "https://github.com/alexmohr/assimilate/pull/1", "https://github.com/alexmohr/assimilate/issues/0"]) {
    const problems = validateAllowlist({ audit: [entry("pug", issue)] });
    assert.equal(problems.length, 1, JSON.stringify(issue));
  }
});

test("a bare package-name string (the frontend format) is rejected", () => {
  assert.ok(validateAllowlist({ audit: ["pug"] }).length > 0);
});

test("duplicate and malformed entries are rejected", () => {
  assert.ok(validateAllowlist({ audit: [entry("pug"), entry("pug")] }).some((p) => p.includes("duplicate")));
  assert.ok(validateAllowlist({ audit: [{ issue: ISSUE }] }).some((p) => p.includes("package")));
  assert.ok(validateAllowlist({}).length > 0);
});

test("a stale allowlist entry warns so its issue can be closed, but does not fail", () => {
  const r = evaluate(clean, { audit: [entry("pug")] });
  assert.equal(r.ok, true);
  assert.equal(r.warnings.length, 1);
  assert.match(r.warnings[0], /"pug".*no longer flagged/);
});

test("an npm audit failure is never read as a clean result", () => {
  for (const audit of [null, {}, { error: { code: "ENOAUDIT", summary: "registry down" } }]) {
    const r = evaluate(audit, { audit: [] });
    assert.equal(r.ok, false);
    assert.match(r.errors.join("\n"), /usable report/);
  }
});

test("issue links must point into the given repo when one is known", () => {
  const own = { audit: [entry("pug", "https://github.com/alexmohr/assimilate/issues/7")] };
  const other = { audit: [entry("pug", "https://github.com/someone/else/issues/7")] };
  assert.deepEqual(validateAllowlist(own, "alexmohr/assimilate"), []);
  assert.deepEqual(validateAllowlist(own, "AlexMohr/Assimilate"), [], "repo match ignores case");
  assert.equal(validateAllowlist(other, "alexmohr/assimilate").length, 1);
  // Without a repo (local runs) any well-formed GitHub issue URL is accepted.
  assert.deepEqual(validateAllowlist(other), []);
  assert.equal(evaluate(clean, other, "alexmohr/assimilate").ok, false);
});

test("the issue link cannot be a prefix trick on another repo", () => {
  const sneaky = { audit: [entry("pug", "https://github.com/alexmohr/assimilate-fork/issues/7")] };
  assert.equal(validateAllowlist(sneaky, "alexmohr/assimilate").length, 1);
});

for (const [name, rel] of [
  ["jscpd", ["..", "..", "jscpd", ".npm-audit-allowlist.json"]],
  ["frontend", ["..", "..", "..", "frontend", ".npm-audit-allowlist.json"]],
]) {
  test(`the committed ${name} allowlist is well-formed and tracks this repo's issues`, () => {
    const allowlist = JSON.parse(fs.readFileSync(path.join(__dirname, ...rel), "utf-8"));
    assert.deepEqual(validateAllowlist(allowlist, "alexmohr/assimilate"), []);
  });
}

test("the frontend and jscpd audit steps both go through this script", () => {
  const ci = fs.readFileSync(path.join(__dirname, "..", "..", "workflows", "ci.yml"), "utf-8");
  const uses = ci.match(/check-npm-audit\.js/g) || [];
  assert.ok(uses.length >= 2, "expected both audit steps to call check-npm-audit.js");
  // The old inline gate must not creep back in next to it.
  assert.doesNotMatch(ci, /ALLOWLIST=\$\(node -e/);
});
