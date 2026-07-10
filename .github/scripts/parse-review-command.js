// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

// Parses the /claude-review manual-retrigger comment command, documented in
// skills/review/SKILL.md. Syntax: `/claude-review` or `/claude-review
// model=<id>` to use a specific model for this run only.

const ALLOWED_MODELS = ["claude-sonnet-5", "claude-opus-4-8", "claude-haiku-4-5"];

module.exports = async ({ core, commentBody, defaultModel }) => {
  const match = /^\/claude-review(?:\s+model=(\S+))?\s*$/m.exec(commentBody.trim());
  if (!match) {
    core.setOutput("matched", "false");
    return;
  }

  const requestedModel = match[1];
  if (requestedModel && !ALLOWED_MODELS.includes(requestedModel)) {
    core.setOutput("matched", "false");
    core.setOutput(
      "error",
      `Unsupported model \`${requestedModel}\`. Allowed: ${ALLOWED_MODELS.join(", ")}.`,
    );
    return;
  }

  core.setOutput("matched", "true");
  core.setOutput("model", requestedModel || defaultModel);
};
