#!/usr/bin/env python3
"""Summarise an upstream commit range for an Ubunye integration PR.

Deterministic (no network, no LLM). Run from the root of an ubunye-creative-* checkout:

    python3 -I ubunye/scripts/upstream_report.py --from <last_sha> --to <new_sha> \
        [--patched-paths ubunye/patched-paths.txt] [--base develop] \
        [--out sync-report.md] [--json sync-report.json]

Classifies commits into release / breaking / security / feature / fix / other, diffs Cargo.lock,
flags new native/git dependencies, finds brand drift and the files that both upstream and
Ubunye changed (conflict hotspots).
"""
from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import tomllib

SECURITY_RE = re.compile(
    r"\b(security|cve-\d|rustsec|ghsa-|vuln|overflow|out[- ]of[- ]bounds|oob|panic|crash|fuzz|"
    r"sanitiz|untrusted|malicious|hostile|dos\b|denial|exhaust|traversal|injection|unsafe)",
    re.I,
)
BREAKING_RE = re.compile(r"(^\w+(\(.+\))?!:|breaking|incompatible|migrat|format version|schema)", re.I)
FEAT_RE = re.compile(r"^(feat(\(.+\))?:|adds?\b|implements?\b|supports?\b|new\b|introduces?\b|enables?\b)", re.I)
FIX_RE = re.compile(
    r"(^(fix(es|ed)?|bug|correct|repair|resolve|revert)\b|\bfix(es)? #\d+|\bno longer\b|\bregression\b|"
    r"\b(can't|cannot|won't|doesn't|don't) (crash|hang|lose|drop|break)|\bwas (wrong|broken|ignored|lost))",
    re.I,
)
TASK_PREFIX_RE = re.compile(r"^([A-Z][A-Z0-9]*(?:-[A-Z0-9]+)+):\s*")
BRAND_RE = re.compile(r"ArtCraft|getartcraft|ai\.storyteller|Learning Machines|storytold", re.I)
BRAND_ALLOW = re.compile(r"(^|/)(LICENSE[^/]*|NOTICE|ATTRIBUTION[^/]*|UPSTREAM[^/]*|credits\.rs)$|^docs/brand/")
PARSER_DIR_RE = re.compile(
    r"^crates/(raw|tiff|meta|codecs|psd|heif|isobmff|matroska|mxf|mpegts|ogg|wav|aac|opus|h264|hevc|"
    r"vp9|av1|prores|dnx|cfb|aaf|omf|captions|lut|icc|zip|pdn|affinity)[^/]*/"
)


def git(*args: str) -> str:
    return subprocess.run(["git", *args], check=True, capture_output=True, text=True).stdout


def lock_packages(rev: str) -> dict[str, set[str]]:
    try:
        text = git("show", f"{rev}:Cargo.lock")
    except subprocess.CalledProcessError:
        return {}
    pkgs: dict[str, set[str]] = {}
    for p in tomllib.loads(text).get("package", []):
        src = p.get("source", "")
        if not src:
            continue  # workspace member
        pkgs.setdefault(p["name"], set()).add(p["version"] + (" (git)" if src.startswith("git+") else ""))
    return pkgs


def workspace_version(rev: str) -> str | None:
    try:
        return tomllib.loads(git("show", f"{rev}:Cargo.toml")).get("workspace", {}).get("package", {}).get("version")
    except (subprocess.CalledProcessError, tomllib.TOMLDecodeError):
        return None


def msrv(rev: str) -> str | None:
    try:
        return tomllib.loads(git("show", f"{rev}:Cargo.toml")).get("workspace", {}).get("package", {}).get("rust-version")
    except (subprocess.CalledProcessError, tomllib.TOMLDecodeError):
        return None


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--from", dest="frm", required=True)
    ap.add_argument("--to", required=True)
    ap.add_argument("--base", default="develop", help="Ubunye branch used for conflict hotspots")
    ap.add_argument("--patched-paths", default=None)
    ap.add_argument("--out", default="sync-report.md")
    ap.add_argument("--json", default=None)
    a = ap.parse_args()
    rng = f"{a.frm}..{a.to}"

    commits = []
    for line in git("log", "--no-merges", "--format=%H%x1f%an%x1f%s", rng).splitlines():
        sha, author, subject = line.split("\x1f", 2)
        commits.append({"sha": sha, "author": author, "subject": subject})
    merges = int(git("rev-list", "--count", "--merges", rng).strip() or 0)

    files_by_commit: dict[str, list[str]] = {}
    for c in commits:
        files_by_commit[c["sha"]] = [f for f in git("diff-tree", "--no-commit-id", "--name-only", "-r", c["sha"]).splitlines() if f]

    cats: dict[str, list[dict]] = {k: [] for k in ("breaking", "security", "feature", "fix", "other")}
    areas: dict[str, int] = {}
    for c in commits:
        m = TASK_PREFIX_RE.match(c["subject"])
        areas[m.group(1) if m else "(none)"] = areas.get(m.group(1) if m else "(none)", 0) + 1
        s = TASK_PREFIX_RE.sub("", c["subject"])
        parser_touch = any(PARSER_DIR_RE.match(f) for f in files_by_commit[c["sha"]])
        if BREAKING_RE.search(s):
            cats["breaking"].append(c)
        elif SECURITY_RE.search(s):
            cats["security"].append(c)
        elif FEAT_RE.search(s):
            cats["feature"].append(c)
        elif FIX_RE.search(s):
            (cats["security"] if parser_touch else cats["fix"]).append(c)
        else:
            cats["other"].append(c)

    changed = sorted(set(git("diff", "--name-only", rng).splitlines()))
    tags = [t for t in git("tag", "--merged", a.to, "--no-merged", a.frm).splitlines() if t]

    old_lock, new_lock = lock_packages(a.frm), lock_packages(a.to)
    added = sorted(n for n in new_lock if n not in old_lock)
    removed = sorted(n for n in old_lock if n not in new_lock)
    bumped = sorted(
        (n, ", ".join(sorted(old_lock[n])), ", ".join(sorted(new_lock[n])))
        for n in new_lock if n in old_lock and new_lock[n] != old_lock[n]
    )
    native = [n for n in added if n.endswith("-sys") or n in {"cc", "cmake", "bindgen", "ring", "aws-lc-sys", "openssl"}]
    gitdeps = [n for n in new_lock if any(v.endswith("(git)") for v in new_lock[n])]

    brand_drift = []
    for f in changed:
        if BRAND_ALLOW.search(f):
            continue
        try:
            diff = git("diff", "-U0", rng, "--", f)
        except subprocess.CalledProcessError:
            continue
        n = sum(1 for l in diff.splitlines() if l.startswith("+") and not l.startswith("+++") and BRAND_RE.search(l))
        if n:
            brand_drift.append((f, n))

    ours: set[str] = set()
    if a.patched_paths:
        try:
            ours |= {l.strip() for l in open(a.patched_paths) if l.strip() and not l.startswith("#")}
        except FileNotFoundError:
            pass
    try:
        ours |= set(git("diff", "--name-only", f"{a.frm}...{a.base}").splitlines())
    except subprocess.CalledProcessError:
        pass
    hotspots = sorted(set(changed) & ours)

    v_old, v_new = workspace_version(a.frm), workspace_version(a.to)
    m_old, m_new = msrv(a.frm), msrv(a.to)
    risk = "high" if (cats["breaking"] or native or gitdeps or hotspots) else "medium" if (cats["security"] or bumped) else "low"

    out = []
    out.append(f"## Upstream integration report\n")
    out.append(f"Range `{a.frm[:12]}..{a.to[:12]}`: **{len(commits)} commits** (+{merges} merges), "
               f"{len({c['author'] for c in commits})} authors, {len(changed)} files changed. Risk: **{risk}**.\n")
    if v_old != v_new:
        out.append(f"- Upstream version: `{v_old}` → `{v_new}`")
    if tags:
        out.append(f"- New upstream tags: {', '.join(f'`{t}`' for t in tags)}")
    if m_old != m_new:
        out.append(f"- ⚠️ MSRV changed: `{m_old}` → `{m_new}` (check rust-toolchain.toml)")
    out.append("")
    titles = {"breaking": "⚠️ Possibly breaking", "security": "🛡️ Security-relevant (keywords or parser crates)",
              "feature": "✨ Features", "fix": "🐛 Fixes", "other": "Other"}
    for k in ("breaking", "security", "feature", "fix", "other"):
        if not cats[k]:
            continue
        out.append(f"<details{' open' if k in ('breaking', 'security') else ''}><summary><b>{titles[k]}</b> ({len(cats[k])})</summary>\n")
        for c in cats[k][:200]:
            out.append(f"- `{c['sha'][:9]}` {c['subject']}")
        if len(cats[k]) > 200:
            out.append(f"- … {len(cats[k]) - 200} more")
        out.append("\n</details>\n")
    top = sorted(areas.items(), key=lambda kv: -kv[1])[:15]
    out.append("### Commits by upstream task area\n")
    out.append(", ".join(f"`{k}` {v}" for k, v in top) + "\n")
    out.append("### Dependencies (Cargo.lock)\n")
    out.append(f"{len(added)} added, {len(removed)} removed, {len(bumped)} changed.")
    if native:
        out.append(f"- ⚠️ New native/C build crates: {', '.join(native)}")
    if gitdeps:
        out.append(f"- ⚠️ Git dependencies (policy violation): {', '.join(gitdeps)}")
    if added:
        out.append(f"- Added: {', '.join(added[:60])}{' …' if len(added) > 60 else ''}")
    if removed:
        out.append(f"- Removed: {', '.join(removed[:60])}{' …' if len(removed) > 60 else ''}")
    for n, o, nv in bumped[:60]:
        out.append(f"- `{n}` {o} → {nv}")
    out.append("\n### Brand drift (new upstream identity strings outside allow-listed files)\n")
    out.append("\n".join(f"- `{f}`: {n} line(s)" for f, n in brand_drift) or "None.")
    out.append("\n### Conflict hotspots (changed by upstream and by Ubunye)\n")
    out.append("\n".join(f"- `{f}`" for f in hotspots) or "None.")
    out.append("\n---\nGenerated by `upstream_report.py`. Merging requires human approval; never force-push.")
    open(a.out, "w").write("\n".join(out) + "\n")

    if a.json:
        json.dump({
            "from": a.frm, "to": a.to, "commits": len(commits), "merges": merges, "files_changed": len(changed),
            "risk": risk, "version": [v_old, v_new], "msrv": [m_old, m_new], "tags": tags,
            "counts": {k: len(v) for k, v in cats.items()}, "areas": areas,
            "deps": {"added": added, "removed": removed, "changed": [b[0] for b in bumped], "native": native, "git": gitdeps},
            "brand_drift": brand_drift, "hotspots": hotspots,
        }, open(a.json, "w"), indent=2)
    print(f"{len(commits)} commits, risk={risk}, report → {a.out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
