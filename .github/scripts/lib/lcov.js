// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

// Minimal LCOV reader - only what analyze-coverage-diff.js needs (per-file
// line hit counts and aggregate line totals), not a general-purpose parser.

function parseLcov(content) {
  const files = new Map(); // path -> Map(line -> hits)
  let current = null;

  for (const rawLine of content.split("\n")) {
    const line = rawLine.trim();
    if (line.startsWith("SF:")) {
      const path = line.slice(3);
      current = files.get(path) || new Map();
      files.set(path, current);
    } else if (line.startsWith("DA:") && current) {
      const [lineNoStr, hitsStr] = line.slice(3).split(",");
      const lineNo = Number(lineNoStr);
      const hits = Number(hitsStr);
      current.set(lineNo, (current.get(lineNo) || 0) + hits);
    } else if (line === "end_of_record") {
      current = null;
    }
  }

  return files;
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

module.exports = { parseLcov, totals };
