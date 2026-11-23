#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr
"""A stand-in for the BorgBase GraphQL API, for the demo and the end-to-end tests.

It answers the four operations the server sends (repoList, sshList, sshAdd, repoAdd) from memory,
accepts the bearer token "demo-token" and refuses every other one with 401. It is never a real
BorgBase: the server is pointed at it with ASSIMILATE_BORGBASE_API_URL.
"""

import json
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer

ACCEPTED_TOKEN = "demo-token"

repos = [
    {"id": "ab12cd34", "name": "laptop", "quota": 500, "quotaEnabled": True,
     "currentUsage": 12.5, "lastModified": "2026-11-20T10:00:00Z"},
    {"id": "ef56ab78", "name": "nas", "quota": None, "quotaEnabled": False,
     "currentUsage": 140.25, "lastModified": "2026-11-21T08:30:00Z"},
]
keys = []


def answer(query, variables):
    if "sshAdd" in query:
        key = {"id": str(len(keys) + 1), "name": variables["name"], "keyData": variables["keyData"]}
        keys.append(key)
        return {"sshAdd": {"keyAdded": key}}
    if "sshList" in query:
        return {"sshList": keys}
    if "repoAdd" in query:
        repo = {"id": "%08x" % (0x10000000 + len(repos)), "name": variables["name"], "quota": None,
                "quotaEnabled": False, "currentUsage": 0, "lastModified": None}
        repos.append(repo)
        return {"repoAdd": {"repoAdded": {"id": repo["id"], "name": repo["name"]}}}
    return {"repoList": repos}


class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        body = json.loads(self.rfile.read(length) or b"{}")
        if self.headers.get("Authorization") != "Bearer " + ACCEPTED_TOKEN:
            self.send_response(401)
            self.end_headers()
            return
        payload = json.dumps(
            {"data": answer(body.get("query", ""), body.get("variables") or {})}
        ).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def log_message(self, fmt, *args):
        sys.stderr.write("borgbase-stand-in: " + fmt % args + "\n")


if __name__ == "__main__":
    HTTPServer(("127.0.0.1", int(sys.argv[1]) if len(sys.argv) > 1 else 8099), Handler).serve_forever()
