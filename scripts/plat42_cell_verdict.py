#!/usr/bin/env python3
"""Turn one cell's cargo log (stdin) into a verdict line. argv: cell rc."""
import json, re, sys
cell, rc = sys.argv[1], int(sys.argv[2])
log = sys.stdin.read()
priv = sorted(set(re.findall(
    r"(?:function|module|field|method|struct|constant|type alias) `([^`]+)` is private", log)))
missing = sorted(set(
    re.findall(r"no (?:method|field|function|associated item) named `([^`]+)`", log)
    + re.findall(r"could not find `([^`]+)`", log)
    + re.findall(r"cannot find (?:function|value|type) `([^`]+)`", log)))
errors = [l for l in log.splitlines() if re.match(r"^(?:\S+:\d+:\d+: )?error(\[E\d+\])?:", l)]
print(json.dumps({"cell": cell, "compiles": rc == 0, "private": priv,
                  "missing": missing, "errors": errors[:6]}))
