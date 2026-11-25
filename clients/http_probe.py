#!/usr/bin/env python3
"""Probe m7s-rs HTTP API (Python standard library only)."""
import json
import sys
import urllib.request

BASE = sys.argv[1] if len(sys.argv) > 1 else "http://127.0.0.1:8180"

for path in ("/health", "/api/streams", "/cluster/api/status/local", "/cluster/api/sessions"):
    try:
        with urllib.request.urlopen(BASE + path, timeout=3) as response:
            print(path, response.status, json.load(response))
    except Exception as exc:
        print(path, "ERROR", exc)
