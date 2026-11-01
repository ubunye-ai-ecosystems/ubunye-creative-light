"""Ubunye-owned. Publish one GitHub release of this app to downloads.thabangvision.com.

    python3 -I ubunye/scripts/publish_downloads.py <light|photo|film> <release id> [--dry-run]

Env: GITHUB_TOKEN (reads the release), INGEST_KEY (the ubunye-creative-ingest Worker's key).
Runs on GitHub's runners (ubunye-release.yml): big files are copied GitHub -> R2 by the Worker, small
ones (.zsync with the renamed AppImage inside, latest.json) are sent to it inline. Nothing passes through
a person's machine. Layout: <app>/<version>/ubunye-creative-<app>-<version>-<platform>,
<app>/stable/ubunye-creative-<app>-linux-<arch>.AppImage(.zsync), <app>/latest.json.
"""
from __future__ import annotations

import base64
import hashlib
import json
import os
import re
import sys
import urllib.error
import urllib.request
from datetime import datetime, timezone

ORG = "ubunye-ai-ecosystems"
BASE = "https://downloads.thabangvision.com"
INGEST = "https://ingest.tools.thabangvision.com/"
UPSTREAM = {"light": "lightcraft", "photo": "photocraft", "film": "filmcraft"}
TYPES = {
    ".msi": "application/x-msi", ".zip": "application/zip", ".dmg": "application/x-apple-diskimage",
    ".AppImage": "application/octet-stream", ".zsync": "application/octet-stream",
    ".deb": "application/vnd.debian.binary-package", ".rpm": "application/x-rpm",
    ".tar.gz": "application/gzip", ".json": "application/json",
}
GH = os.environ.get("GITHUB_TOKEN", "")
KEY = os.environ.get("INGEST_KEY", "")
UA = "ubunye-publish/2"


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *a, **k):
        return None


def gh(path: str):
    req = urllib.request.Request("https://api.github.com" + path, headers={
        "Authorization": "Bearer " + GH, "Accept": "application/vnd.github+json", "User-Agent": UA})
    with urllib.request.urlopen(req, timeout=60) as r:
        return json.loads(r.read())


def signed_url(repo: str, asset_id: int) -> str:
    req = urllib.request.Request(f"https://api.github.com/repos/{ORG}/{repo}/releases/assets/{asset_id}", headers={
        "Authorization": "Bearer " + GH, "Accept": "application/octet-stream", "User-Agent": UA})
    try:
        urllib.request.build_opener(NoRedirect).open(req, timeout=60)
    except urllib.error.HTTPError as e:
        if e.code in (301, 302, 307):
            return e.headers["Location"]
        raise
    raise RuntimeError("expected a redirect to the asset's storage URL")


def ctype(name: str) -> str:
    return next((t for ext, t in TYPES.items() if name.endswith(ext)), "application/octet-stream")


def post(job: dict, what: str, dry: bool) -> None:
    print(f"  {what}", flush=True)
    if dry:
        return
    body = json.dumps(job).encode()
    for attempt in range(3):
        req = urllib.request.Request(INGEST, data=body, method="POST", headers={
            "x-ingest-key": KEY, "Content-Type": "application/json", "User-Agent": UA})
        try:
            with urllib.request.urlopen(req, timeout=900) as r:
                out = json.loads(r.read())
            want = job.get("size")
            if want is None or out.get("size") == want:
                return
            raise RuntimeError(f"stored size {out.get('size')} != {want}")
        except Exception as e:  # noqa: BLE001
            print(f"    attempt {attempt + 1} failed: {str(e)[:200]}", flush=True)
    sys.exit(f"could not store {job['key']}")


def copy_big(url: str, key: str, size: int, name: str, dry: bool) -> None:
    post({"url": url, "key": key, "size": size, "contentType": ctype(name)}, f"copy {key} ({size // 1048576} MB)", dry)


def put_small(key: str, data: bytes, name: str, dry: bool) -> None:
    post({"body": base64.b64encode(data).decode(), "key": key, "contentType": ctype(name)}, f"put {key}", dry)


def rewrite_zsync(data: bytes, appimage_name: str) -> bytes:
    """Point the .zsync header (Filename, URL) at the renamed AppImage; the block data is unchanged."""
    head, sep, body = data.partition(b"\n\n")
    lines = []
    for line in head.split(b"\n"):
        if line.startswith(b"Filename: ") or line.startswith(b"URL: "):
            line = line.split(b": ", 1)[0] + b": " + appimage_name.encode()
        lines.append(line)
    return b"\n".join(lines) + sep + body


def main() -> int:
    app, release_id = sys.argv[1], sys.argv[2]
    dry = "--dry-run" in sys.argv
    if app not in UPSTREAM:
        sys.exit(f"unknown app {app}")
    if not GH or (not KEY and not dry):
        sys.exit("GITHUB_TOKEN and INGEST_KEY are required")
    up, repo = UPSTREAM[app], f"ubunye-creative-{app}"
    release = gh(f"/repos/{ORG}/{repo}/releases/{release_id}")
    pat = re.compile(rf"^{up}(-cli)?-(\d+\.\d+\.\d+(?:-(?:rc|alpha|beta|pre)[0-9A-Za-z.]*)?)-(.+)$")
    assets = [a for a in release["assets"] if pat.match(a["name"]) and "-web-" not in a["name"]]
    versions = {pat.match(a["name"]).group(2) for a in assets}
    if len(versions) != 1:
        sys.exit(f"expected one version in the release, found {versions or 'none'}")
    version = versions.pop()
    manifest = {"app": app, "version": version,
                "published": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"), "files": []}
    for a in sorted(assets, key=lambda a: a["name"]):
        m = pat.match(a["name"])
        name = f"ubunye-creative-{app}{m.group(1) or ''}-{version}-{m.group(3)}"
        key = f"{app}/{version}/{name}"
        am = re.match(r"^ubunye-creative-\w+-[^-]+-linux-(\w+)\.AppImage(\.zsync)?$", name)
        stable = f"ubunye-creative-{app}-linux-{am.group(1)}.AppImage{am.group(2) or ''}" if am else None
        if name.endswith(".zsync"):
            with urllib.request.urlopen(signed_url(repo, a["id"]), timeout=120) as r:
                raw = r.read()
            data = rewrite_zsync(raw, name[: -len(".zsync")])
            put_small(key, data, name, dry)
            if stable:
                put_small(f"{app}/stable/{stable}", rewrite_zsync(raw, stable[: -len(".zsync")]), stable, dry)
            size, digest = len(data), hashlib.sha256(data).hexdigest()
        else:
            copy_big(signed_url(repo, a["id"]), key, a["size"], name, dry)
            if stable:
                copy_big(signed_url(repo, a["id"]), f"{app}/stable/{stable}", a["size"], name, dry)
            size, digest = a["size"], (a.get("digest") or "").removeprefix("sha256:")
        manifest["files"].append({"name": name, "url": f"{BASE}/{key}", "size": size, "sha256": digest})
    put_small(f"{app}/latest.json", (json.dumps(manifest, indent=2) + "\n").encode(), "latest.json", dry)
    print(f"published {app} {version}: {len(manifest['files'])} files")
    return 0


if __name__ == "__main__":
    sys.exit(main())
