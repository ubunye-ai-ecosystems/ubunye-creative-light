#!/usr/bin/env python3
"""Smoke test of the LIVE Ubunye app: what downloads.thabangvision.com serves right now.

    python ubunye/scripts/live_smoke.py --app light --out OUT_DIR

1. Read https://downloads.thabangvision.com/<app>/latest.json and download this platform's installers
   (Linux: .deb and AppImage; Windows: .msi and portable .zip; macOS: .dmg), checking size and sha256.
2. Install them the way a user does (apt, msiexec, the DMG's .app).
3. Check the installed identity: name, version = latest.json, no upstream names in what the user sees,
   and the updater/product links inside the shipped binary point at thabangvision.com, not upstream.
4. Run the installed app on real sample images (a Sony a7 III raw and a camera JPEG, CC0, pinned by
   sha256) and check every output exists, is non-empty and decodes (Pillow), at the size asked for.

Writes OUT_DIR/summary.json and the outputs; exits non-zero on the first failed check. Standard
library plus Pillow. The app-specific parts are in APPS and run_<app>; Photo and Film copy this file
and add theirs.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.request
import zipfile
from pathlib import Path

UA = "ubunye-live-smoke/1.0 (+https://github.com/ubunye-ai-ecosystems)"
DOWNLOADS = "https://downloads.thabangvision.com"

# Real sample images, CC0, pinned. Internal format tests only: never shown as product demos.
MEDIA = {
    "a7iii.ARW": (
        "https://raw.pixls.us/download/data/Sony/ILCE-7M3/_DSC0009.ARW",  # raw.pixls.us, CC0
        "250784580ea527442c09004417bb0eead484f2bf3ee8f9121a776ac65bb50d0f",
    ),
    "church.jpg": (
        # Wikimedia Commons, "! Russisch-Orthodoxe Kirche (Dresden) Zwiebeltürme.JPG", CC0
        "https://upload.wikimedia.org/wikipedia/commons/1/1f/%21_Russisch-Orthodoxe_Kirche_%28Dresden%29_Zwiebelt%C3%BCrme.JPG",
        "71cb618a15f342b38f0d5b7316a7f82c834d72b52caea38b8135e50cadfb39d9",
    ),
}

APPS = {
    "light": {
        "name": "Ubunye Light",
        "app_id": "org.ubunye.creative.light",
        "deb_package": "lightcraft",  # nfpm keeps the binary name
        "product_page": "https://www.thabangvision.com/tools/light",
        "repo": "ubunye-ai-ecosystems/ubunye-creative-light",
        "upstream_app_id": "ai.storyteller.lightcraft",
        "upstream_words": ["LightCraft", "ArtCraft", "storytold", "getartcraft"],
        # the credit the brand requires (README, About, --help): allowed
        "credit": "Based on LightCraft by the ArtCraft team",
        "win_key": r"Software\Ubunye AI Ecosystems\Ubunye Light",
        "mac_bundle": "Ubunye Light.app",
        "mac_exe": "LightCraft",
        "gui": "lightcraft",
        "cli": "lightcraft-cli",
    },
}

RESULTS: list[dict] = []


class Fail(Exception):
    pass


def check(ok: bool, what: str, detail: str = "") -> None:
    RESULTS.append({"check": what, "ok": bool(ok), "detail": detail[:500]})
    print(("PASS " if ok else "FAIL ") + what + (f": {detail}" if detail else ""), flush=True)
    if not ok:
        raise Fail(what + (f": {detail}" if detail else ""))


def fetch(url: str, dest: Path | None = None) -> bytes | None:
    req = urllib.request.Request(url, headers={"User-Agent": UA})
    for attempt in range(4):
        try:
            with urllib.request.urlopen(req, timeout=300) as r:
                if dest is None:
                    return r.read()
                with open(dest, "wb") as fh:
                    shutil.copyfileobj(r, fh, 1 << 20)
                return None
        except Exception as e:  # network blips on runners: retry
            if attempt == 3:
                raise
            print(f"retry {url}: {e}", flush=True)
            time.sleep(5 * (attempt + 1))
    return None


def sha256(p: Path) -> str:
    h = hashlib.sha256()
    with open(p, "rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def run(cmd: list, env: dict | None = None, timeout: int = 900, check_rc: bool = True) -> subprocess.CompletedProcess:
    print("$ " + " ".join(str(c) for c in cmd), flush=True)
    p = subprocess.run([str(c) for c in cmd], capture_output=True, text=True, encoding="utf-8", errors="replace",
                       env=env, timeout=timeout)
    if p.stdout.strip():
        print(p.stdout[-4000:], flush=True)
    if p.stderr.strip():
        print(p.stderr[-4000:], file=sys.stderr, flush=True)
    if check_rc and p.returncode != 0:
        check(False, "command succeeds", f"{cmd[0]} exited {p.returncode}")
    return p


def os_name() -> str:
    s = platform.system()
    return {"Linux": "linux", "Windows": "windows", "Darwin": "macos"}[s]


# ---------------------------------------------------------------------------------------------
# download + identity helpers
# ---------------------------------------------------------------------------------------------
def download_live(app: str, work: Path) -> tuple[dict, dict[str, Path]]:
    latest = json.loads(fetch(f"{DOWNLOADS}/{app}/latest.json"))
    version = latest["version"]
    print(f"live {app} {version}, published {latest.get('published')}", flush=True)
    want = {
        "linux": (".deb", ".AppImage"),
        "windows": (".msi", "-portable.zip"),
        "macos": (".dmg",),
    }[os_name()]
    files: dict[str, Path] = {}
    for f in latest["files"]:
        suffix = next((w for w in want if f["name"].endswith(w)), None)
        if not suffix or ("x86_64" not in f["name"] and "x64" not in f["name"] and "universal" not in f["name"]):
            continue
        dest = work / f["name"]
        fetch(f["url"], dest)
        check(dest.stat().st_size == f["size"], f"download size {f['name']}", f"{dest.stat().st_size} vs {f['size']}")
        if f.get("sha256"):
            check(sha256(dest) == f["sha256"], f"download sha256 {f['name']}")
        check(version in f["name"], f"file name carries the live version {f['name']}")
        files[suffix] = dest
    for w in want:
        check(w in files, f"latest.json lists a {w} for {os_name()}")
    return latest, files


def no_upstream_in_text(cfg: dict, text: str, where: str) -> None:
    cleaned = text.replace(cfg["credit"], "")
    # case-sensitive: the binaries keep upstream's lower-case names (`lightcraft`, `lightcraft-cli`)
    found = [w for w in cfg["upstream_words"] if w in cleaned]
    check(not found, f"no upstream names in {where}", ", ".join(found))


def binary_identity(cfg: dict, exe: Path) -> None:
    data = exe.read_bytes()
    for must in (cfg["product_page"], cfg["repo"], cfg["app_id"]):
        check(must.encode() in data, f"{exe.name} carries {must}")
    check(cfg["upstream_app_id"].encode() not in data, f"{exe.name} has no upstream app id {cfg['upstream_app_id']}")
    # informational: upstream strings that may legitimately remain (the credit link, library internals)
    for w in (b"storytold/", b"getartcraft.com"):
        RESULTS.append({"check": f"info: occurrences of {w.decode()} in {exe.name}", "ok": True,
                        "detail": str(data.count(w))})


def cli_identity(cfg: dict, cli: Path, version: str, env: dict) -> None:
    out = run([cli, "--version"], env=env).stdout
    check(version in out and cfg["name"] in out, f"{cli.name} --version names {cfg['name']} {version}", out.strip())
    help_text = run([cli, "--help"], env=env, check_rc=False)
    no_upstream_in_text(cfg, help_text.stdout + help_text.stderr, f"{cli.name} --help")


def decode(p: Path, long_edge: int | None) -> dict:
    from PIL import Image

    check(p.is_file() and p.stat().st_size > 0, f"output exists and is non-empty: {p.name}",
          str(p.stat().st_size) if p.exists() else "missing")
    with Image.open(p) as im:
        im.load()
        w, h = im.size
        info = {"file": p.name, "bytes": p.stat().st_size, "width": w, "height": h, "mode": im.mode, "format": im.format}
    check(w > 0 and h > 0, f"output decodes: {p.name}", json.dumps(info))
    if long_edge:
        check(max(w, h) == long_edge, f"output long edge {long_edge}: {p.name}", f"{w}x{h}")
    RESULTS.append({"check": f"output {p.name}", "ok": True, "detail": json.dumps(info)})
    return info


def media(work: Path) -> dict[str, Path]:
    out = {}
    d = work / "media"
    d.mkdir(exist_ok=True)
    for name, (url, digest) in MEDIA.items():
        p = d / name
        fetch(url, p)
        check(sha256(p) == digest, f"sample image sha256 {name}")
        out[name] = p
    return out


# ---------------------------------------------------------------------------------------------
# Light
# ---------------------------------------------------------------------------------------------
def light_cli_images(cfg: dict, cli: Path, imgs: dict[str, Path], out: Path, tag: str, env: dict) -> None:
    arw, jpg = imgs["a7iii.ARW"], imgs["church.jpg"]
    jobs = [
        (arw, out / f"{tag}-raw.jpg", ["--size", "2048"], 2048),
        (arw, out / f"{tag}-raw-edit.png", ["--size", "1600", "--set", "light.exposure=0.5"], 1600),
        (jpg, out / f"{tag}-jpg-edit.webp", ["--size", "1600", "--set", "light.exposure=-0.3"], 1600),
    ]
    for src, dst, opts, edge in jobs:
        run([cli, "render", src, "-o", dst, *opts], env=env)
        decode(dst, edge)
    # the library path users take: import both, then export them with an edit
    lib = out.parent / f"lib-{tag}"
    shutil.rmtree(lib, ignore_errors=True)
    export_dir = out / f"{tag}-library-export"
    export_dir.mkdir(exist_ok=True)
    p = run([cli, "run", "--library", lib, "--import", arw, "--import", jpg, "catalog.query",
             "develop.set", "control=light.exposure", "value=0.4",
             "app.export", f"dir={export_dir}", "longEdge=1200"], env=env)
    check(len([l for l in p.stdout.splitlines() if l.strip()]) >= 3, f"library run answers every command ({tag})", p.stdout[-300:])
    exported = sorted(export_dir.iterdir())
    check(len(exported) == 2, f"library import + export wrote both photos ({tag})", ", ".join(x.name for x in exported))
    for f in exported:
        decode(f, 1200)


def light_linux(cfg: dict, latest: dict, files: dict, imgs: dict, out: Path, env: dict) -> None:
    version = latest["version"]
    run(["sudo", "apt-get", "update", "-qq"])
    run(["sudo", "apt-get", "install", "-y", "-qq", str(files[".deb"])], timeout=1800)
    q = run(["dpkg-query", "-W", "-f=${Package} ${Version} ${Maintainer}", cfg["deb_package"]]).stdout
    check(version in q, "deb package version = latest.json", q)
    no_upstream_in_text(cfg, q, "deb package metadata")
    desktop = Path(f"/usr/share/applications/{cfg['app_id']}.desktop")
    check(desktop.is_file(), f"desktop entry {desktop}")
    text = desktop.read_text(encoding="utf-8")
    check(f"Name={cfg['name']}" in text, "desktop entry names the app", text[:200])
    no_upstream_in_text(cfg, text, "desktop entry")
    gui = Path(shutil.which(cfg["gui"]) or "")
    cli = Path(shutil.which(cfg["cli"]) or "")
    check(gui.is_file() and cli.is_file(), "deb installs the app and the CLI on the PATH", f"{gui} {cli}")
    out_gui = run([gui, "--version"], env=env).stdout
    check(version in out_gui and cfg["name"] in out_gui, "app --version", out_gui.strip())
    binary_identity(cfg, gui)
    cli_identity(cfg, cli, version, env)
    light_cli_images(cfg, cli, imgs, out, "deb", env)
    # AppImage: the same payload, unpacked (no FUSE on runners)
    ai = files[".AppImage"]
    ai.chmod(0o755)
    root = out.parent / "appimage"
    shutil.rmtree(root, ignore_errors=True)
    root.mkdir()
    subprocess.run([str(ai), "--appimage-extract"], cwd=root, check=True, capture_output=True)
    acli = root / "squashfs-root" / "usr" / "bin" / cfg["cli"]
    check(acli.is_file(), "AppImage carries the CLI", str(acli))
    cli_identity(cfg, acli, version, env)
    binary_identity(cfg, root / "squashfs-root" / "usr" / "bin" / cfg["gui"])
    light_cli_images(cfg, acli, imgs, out, "appimage", env)


def win_uninstall_entry(name: str) -> dict:
    import winreg

    for view in (winreg.KEY_WOW64_64KEY, winreg.KEY_WOW64_32KEY):
        for hive in (winreg.HKEY_LOCAL_MACHINE, winreg.HKEY_CURRENT_USER):
            try:
                root = winreg.OpenKey(hive, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall", 0, winreg.KEY_READ | view)
            except OSError:
                continue
            for i in range(winreg.QueryInfoKey(root)[0]):
                sub = winreg.OpenKey(root, winreg.EnumKey(root, i))
                try:
                    if winreg.QueryValueEx(sub, "DisplayName")[0] == name:
                        vals = {}
                        for k in ("DisplayName", "DisplayVersion", "Publisher", "InstallLocation", "URLInfoAbout", "HelpLink"):
                            try:
                                vals[k] = str(winreg.QueryValueEx(sub, k)[0])
                            except OSError:
                                pass
                        return vals
                except OSError:
                    pass
    return {}


def win_install_dir(key: str) -> Path | None:
    import winreg

    for hive in (winreg.HKEY_LOCAL_MACHINE, winreg.HKEY_CURRENT_USER):
        for view in (winreg.KEY_WOW64_64KEY, winreg.KEY_WOW64_32KEY):
            try:
                k = winreg.OpenKey(hive, key, 0, winreg.KEY_READ | view)
                for v in ("InstallDir", "InstallFolder"):
                    try:
                        return Path(winreg.QueryValueEx(k, v)[0])
                    except OSError:
                        pass
            except OSError:
                pass
    return None


def light_windows(cfg: dict, latest: dict, files: dict, imgs: dict, out: Path, env: dict) -> None:
    version = latest["version"]
    log = out / "msi-install.log"
    run(["msiexec", "/i", str(files[".msi"]), "/qn", "/norestart", "/l*v", str(log)], timeout=1800)
    entry = win_uninstall_entry(cfg["name"])
    check(bool(entry), f"installed: Apps & features lists {cfg['name']}", json.dumps(entry))
    check(entry.get("DisplayVersion", "").startswith(version), "installed version = latest.json", json.dumps(entry))
    no_upstream_in_text(cfg, json.dumps(entry), "Apps & features entry")
    d = win_install_dir(cfg["win_key"])
    check(d is not None and (d / f"{cfg['cli']}.exe").is_file(), f"install folder from HKLM\\{cfg['win_key']}", str(d))
    gui, cli = d / f"{cfg['gui']}.exe", d / f"{cfg['cli']}.exe"
    binary_identity(cfg, gui)
    cli_identity(cfg, cli, version, env)
    light_cli_images(cfg, cli, imgs, out, "msi", env)
    # portable zip
    pz = out.parent / "portable"
    shutil.rmtree(pz, ignore_errors=True)
    with zipfile.ZipFile(files["-portable.zip"]) as z:
        z.extractall(pz)
    pcli = next(pz.rglob(f"{cfg['cli']}.exe"), None)
    check(pcli is not None, "portable zip carries the CLI")
    cli_identity(cfg, pcli, version, env)
    light_cli_images(cfg, pcli, imgs, out, "portable", env)
    run(["msiexec", "/x", str(files[".msi"]), "/qn", "/norestart", "/l*v", str(out / "msi-uninstall.log")], timeout=1800)
    check(not win_uninstall_entry(cfg["name"]), "uninstall removes the app")


class Control:
    """The app's JSON-lines control channel (docs/control-protocol.md)."""

    def __init__(self, port: int):
        self.s = socket.create_connection(("127.0.0.1", port), timeout=300)
        self.f = self.s.makefile("rw", encoding="utf-8", newline="\n")
        self.n = 0

    def call(self, method: str, params: dict | None = None):
        self.n += 1
        self.f.write(json.dumps({"id": self.n, "method": method, "params": params or {}}) + "\n")
        self.f.flush()
        line = self.f.readline()
        reply = json.loads(line)
        check(reply.get("ok") is True, f"control {method} {(params or {}).get('command', '')}".strip(), line[:300])
        return reply.get("result")

    def run(self, command: str, params: dict | None = None):
        return self.call("engine.execute", {"command": command, "params": params or {}})


def light_macos(cfg: dict, latest: dict, files: dict, imgs: dict, out: Path, env: dict) -> None:
    import plistlib

    version = latest["version"]
    mnt = out.parent / "dmg"
    run(["hdiutil", "attach", "-nobrowse", "-readonly", "-mountpoint", mnt, files[".dmg"]])
    try:
        src = mnt / cfg["mac_bundle"]
        check(src.is_dir(), f"DMG holds {cfg['mac_bundle']}", ", ".join(x.name for x in mnt.iterdir()))
        app = out.parent / cfg["mac_bundle"]
        shutil.rmtree(app, ignore_errors=True)
        run(["ditto", src, app])
    finally:
        run(["hdiutil", "detach", mnt], check_rc=False)
    run(["codesign", "--verify", "--deep", "--strict", app], check_rc=False)
    info = plistlib.loads((app / "Contents" / "Info.plist").read_bytes())
    check(info.get("CFBundleIdentifier") == cfg["app_id"], "bundle id", str(info.get("CFBundleIdentifier")))
    check(info.get("CFBundleShortVersionString") == version, "bundle version = latest.json", str(info.get("CFBundleShortVersionString")))
    check(info.get("CFBundleDisplayName") == cfg["name"], "bundle display name", str(info.get("CFBundleDisplayName")))
    shown = {k: v for k, v in info.items() if isinstance(v, str) and k not in ("CFBundleExecutable", "CFBundleIconFile")}
    no_upstream_in_text(cfg, json.dumps(shown), "Info.plist (what Finder and macOS show)")
    exe = app / "Contents" / "MacOS" / cfg["mac_exe"]
    v = run([exe, "--version"], env=env).stdout
    check(version in v and cfg["name"] in v, "app --version", v.strip())
    binary_identity(cfg, exe)
    # the installed app itself, driven over its control channel (no CLI ships in the DMG)
    port = 47980
    lib = out.parent / "lib-macos"
    shutil.rmtree(lib, ignore_errors=True)
    log = open(out / "app-stderr.log", "w")
    proc = subprocess.Popen([str(exe), "--library", str(lib), "--no-demo", "--control", str(port)], env=env,
                            stdout=log, stderr=subprocess.STDOUT)
    try:
        deadline = time.time() + 180
        while True:
            try:
                c = Control(port)
                break
            except OSError:
                check(proc.poll() is None, "app keeps running while it starts", f"exit {proc.poll()}")
                if time.time() > deadline:
                    check(False, "app opens its control port", "timed out")
                time.sleep(2)
        r = c.run("library.import", {"paths": [str(imgs["a7iii.ARW"]), str(imgs["church.jpg"])]})
        ids = list(r.get("imported", []))
        check(len(ids) == 2, "the app imports both images", json.dumps(r)[:300])
        exports = []
        for i, pid in enumerate(ids):
            c.run("library.select", {"ids": [pid], "active": pid})
            c.run("develop.set", {"values": {"light.exposure": 0.4}})
            dst = out / f"app-export-{i}.jpg"
            res = c.run("app.export", {"ids": [pid], "path": str(dst), "longEdge": 1600})
            exports.append(res)
            decode(dst, 1600)
        shot = out / "app-ui.png"
        c.call("ui.screenshot", {"path": str(shot)})
        decode(shot, None)
    finally:
        proc.terminate()
        try:
            proc.wait(20)
        except subprocess.TimeoutExpired:
            proc.kill()
        log.close()


RUNNERS = {"light": {"linux": light_linux, "windows": light_windows, "macos": light_macos}}


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--app", required=True, choices=sorted(APPS))
    ap.add_argument("--out", required=True)
    a = ap.parse_args()
    cfg = APPS[a.app]
    out = Path(a.out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    work = Path(tempfile.mkdtemp(prefix="live-smoke-"))
    env = dict(os.environ)
    env["LIGHTCRAFT_CONFIG_DIR"] = str(work / "config")  # isolated profile (docs/control-protocol.md)
    summary = {"app": a.app, "os": os_name(), "started": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())}
    rc = 0
    try:
        latest, files = download_live(a.app, work)
        summary["version"] = latest["version"]
        summary["files"] = [p.name for p in files.values()]
        imgs = media(work)
        RUNNERS[a.app][os_name()](cfg, latest, files, imgs, out, env)
    except Fail as e:
        summary["failed"] = str(e)
        rc = 1
    except Exception as e:  # anything unexpected is a failure too, with its message
        summary["failed"] = f"{type(e).__name__}: {e}"
        print(f"FAIL unexpected: {summary['failed']}", flush=True)
        rc = 1
    summary["checks"] = RESULTS
    summary["passed"] = rc == 0
    (out / "summary.json").write_text(json.dumps(summary, indent=2), encoding="utf-8")
    step = os.environ.get("GITHUB_STEP_SUMMARY")
    if step:
        with open(step, "a", encoding="utf-8") as fh:
            fh.write(f"### {cfg['name']} {summary.get('version', '?')} live smoke on {os_name()}: "
                     f"{'PASS' if rc == 0 else 'FAIL'}\n\n")
            for r in RESULTS:
                fh.write(f"- {'✅' if r['ok'] else '❌'} {r['check']}" + (f" ({r['detail'][:160]})" if r["detail"] else "") + "\n")
            if summary.get("failed"):
                fh.write(f"\n**Failed:** {summary['failed']}\n")
    print(f"\n{'PASS' if rc == 0 else 'FAIL'}: {sum(r['ok'] for r in RESULTS)} checks passed"
          + (f"; failed: {summary.get('failed')}" if rc else ""), flush=True)
    return rc


if __name__ == "__main__":
    sys.exit(main())
