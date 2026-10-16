#!/usr/bin/env python3
"""Control-channel client for a running TexCraft (see docs/control-protocol.md).

    tc.py METHOD '{json params}' [METHOD '{json params}' ...]

Port and token come from TEXCRAFT_PORT (default 47900) and TEXCRAFT_TOKEN_FILE (default
/tmp/texcraft-dev/token), as written by run-dev-app.sh. Prints one line per call.
"""
import json
import os
import socket
import sys

port = int(os.environ.get("TEXCRAFT_PORT", "47900"))
token_file = os.environ.get("TEXCRAFT_TOKEN_FILE", "/tmp/texcraft-dev/token")
token = open(token_file).read().strip()
conn = socket.create_connection(("127.0.0.1", port), timeout=60)
f = conn.makefile("rw")


def call(i, method, params):
    f.write(json.dumps({"id": i, "method": method, "params": params}) + "\n")
    f.flush()
    return json.loads(f.readline())


r = call(0, "auth", {"token": token})
if not r.get("ok"):
    sys.exit(f"auth failed: {r}")
args = sys.argv[1:]
for k in range(0, len(args), 2):
    method = args[k]
    params = json.loads(args[k + 1]) if k + 1 < len(args) else {}
    r = call(k + 1, method, params)
    out = r.get("result") if r.get("ok") else {"ERROR": r.get("error")}
    print(f"{method}: {json.dumps(out, ensure_ascii=False)}")
