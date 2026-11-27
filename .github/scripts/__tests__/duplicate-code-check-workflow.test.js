// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

const test = require("node:test");
const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { join } = require("node:path");

/**
 * duplicate-code-check.yml runs under `pull_request_target` with write
 * tokens, so nothing in the untrusted PR tree may ever be executed or trusted
 * as configuration. These checks pin the pieces of that guarantee a future
 * edit could silently break (see #732).
 */
const WORKFLOW = readFileSync(
  join(__dirname, "..", "..", "workflows", "duplicate-code-check.yml"),
  "utf-8",
);

/** Split the workflow into its `- name:` steps. */
function steps() {
  return WORKFLOW.split(/^\s+- name: /m)
    .slice(1)
    .map((raw) => ({ name: raw.split("\n")[0].trim(), body: raw }));
}

function step(namePrefix) {
  const found = steps().find((s) => s.name.startsWith(namePrefix));
  assert.ok(found, `no step named "${namePrefix}..." in duplicate-code-check.yml`);
  return found;
}

/** The workflow with comment lines removed, so prose can't trip the checks. */
const CODE = WORKFLOW.split("\n")
  .filter((line) => !line.trim().startsWith("#"))
  .join("\n");

test("the workflow is the privileged pull_request_target kind", () => {
  // If this stops being true the remaining checks guard the wrong thing.
  assert.match(CODE, /pull_request_target:/);
});

test("the default-branch checkout leaves no token behind in .git/config", () => {
  const checkout = step("Checkout scripts from default branch");
  assert.match(checkout.body, /persist-credentials:\s*false/);
});

test("every checkout drops its credentials", () => {
  const checkouts = steps().filter((s) => /uses:\s*actions\/checkout@/.test(s.body));
  assert.ok(checkouts.length >= 2, "expected both checkouts");
  for (const c of checkouts) {
    assert.match(
      c.body,
      /persist-credentials:\s*false/,
      `checkout step "${c.name}" keeps credentials`,
    );
  }
});

test("the default-branch checkout supplies the trusted jscpd install and config", () => {
  const checkout = step("Checkout scripts from default branch");
  assert.match(checkout.body, /ref:\s*\$\{\{\s*github\.event\.repository\.default_branch\s*\}\}/);
  for (const path of [".github/scripts", ".github/jscpd", ".jscpd.json"]) {
    assert.ok(
      checkout.body.includes(path),
      `default-branch sparse checkout is missing ${path}`,
    );
  }
});

test("the PR head is checked out into pr-src, not the workspace root", () => {
  const checkout = step("Checkout PR head");
  assert.match(checkout.body, /path:\s*pr-src\b/);
});

test("jscpd is installed from the trusted directory, with scripts disabled", () => {
  const install = step("Install jscpd");
  assert.match(install.body, /working-directory:\s*\.github\/jscpd\s*$/m);
  assert.match(install.body, /npm ci\b[^\n]*--ignore-scripts/);
});

test("no package manager runs inside the PR tree", () => {
  // `working-directory: pr-src` is fine for the scan itself, but only for a
  // step that never invokes npm/npx/yarn/pnpm.
  for (const s of steps()) {
    if (/working-directory:\s*pr-src\b/.test(s.body)) {
      assert.doesNotMatch(
        s.body.replace(/^\s*#.*$/gm, ""),
        /\b(npx|npm|yarn|pnpm)\b/,
        `step "${s.name}" runs a package manager in pr-src`,
      );
    }
  }
  assert.doesNotMatch(CODE, /\bnpx\b/, "the workflow must not use npx");
});

test("jscpd runs from the pinned install with the default branch's config", () => {
  const run = step("Run jscpd");
  assert.match(
    run.body,
    /\$GITHUB_WORKSPACE\/\.github\/jscpd\/node_modules\/jscpd\/run-jscpd\.js/,
  );
  // The config must be an absolute path under the workspace root (the default
  // branch checkout) - a relative path would resolve into pr-src and let the
  // PR rewrite its own ignore list.
  const config = run.body.match(/--config\s+"?([^"\s\\]+)"?/);
  assert.ok(config, "jscpd is not passed --config");
  assert.ok(
    config[1].startsWith("$GITHUB_WORKSPACE/") && !config[1].includes("pr-src"),
    `--config must live under $GITHUB_WORKSPACE, got ${config[1]}`,
  );
});

test("the analysis script is loaded from the default-branch workspace", () => {
  const analyze = step("Analyze duplication");
  assert.match(
    analyze.body,
    /\$\{process\.env\.GITHUB_WORKSPACE\}\/\.github\/scripts\/analyze-duplication\.js/,
  );
  assert.doesNotMatch(analyze.body, /pr-src\/\.github/);
});
