#!/usr/bin/env python3
"""Writes the update manifest (latest.json) for a release.

    latest_json.py VERSION TAG OWNER/REPO DIST_DIR NOTES_FILE

DIST_DIR holds the release archives: TexCraft-<version>-macos.zip and
TexCraft-<version>-windows-x64.zip. The app reads the manifest from
https://github.com/OWNER/REPO/releases/latest/download/latest.json (see crates/update).
"""
import datetime
import hashlib
import json
import os
import sys

version, tag, repo, dist, notes_file = sys.argv[1:6]
notes = open(notes_file, encoding="utf-8").read().strip() if os.path.exists(notes_file) else ""
base = f"https://github.com/{repo}/releases/download/{tag}"
platforms = {}
for key, name in [
    ("macos", f"TexCraft-{version}-macos.zip"),
    ("windows-x86_64", f"TexCraft-{version}-windows-x64.zip"),
]:
    path = os.path.join(dist, name)
    if not os.path.exists(path):
        sys.exit(f"missing {path}")
    data = open(path, "rb").read()
    platforms[key] = {
        "url": f"{base}/{name}",
        "sha256": hashlib.sha256(data).hexdigest(),
        "size": len(data),
    }
manifest = {
    "version": version,
    "tag": tag,
    "pub_date": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "notes": notes,
    "release_url": f"https://github.com/{repo}/releases/tag/{tag}",
    "platforms": platforms,
}
with open(os.path.join(dist, "latest.json"), "w", encoding="utf-8") as f:
    json.dump(manifest, f, ensure_ascii=False, indent=2)
print(json.dumps(manifest, ensure_ascii=False, indent=2))
