#!/usr/bin/env python3
"""Apply the Ubunye Creative Light brand to the upstream LightCraft tree, or check that it is applied.

    python -I ubunye/brand/apply.py           # (re)apply every scripted brand change
    python -I ubunye/brand/apply.py --check   # exit 1 and list brand leaks or drift, change nothing

Python 3.11+ standard library only. Every step is idempotent: running it twice changes nothing the
second time, so it is safe to rerun after every upstream merge.

What it does, in order (see ubunye/REBRANDING.md for the why):
  1. writes crates/engine/src/brand.rs from ubunye/brand.toml (the central brand constants);
  2. applies the exact text rules below (each written against UPSTREAM text, so after a merge that
     brought the upstream line back the rule simply applies again);
  3. replaces upstream ids, vendor and links across the tree (outside the allow-list);
  4. rewrites the product name "LightCraft" inside Rust string literals (never in comments or
     identifiers) and in the UI message catalogs, keys and values alike, so lookups keep matching;
     a few upstream labels about ArtCraft/Discord are renamed, with our translations from
     ubunye/i18n/overlay.json;
  5. renames packaging files and hicolor icons to the new app id, copies our icon SVG over the
     upstream icon sources and deletes the ArtCraft logo files;
  6. rewrites the README header and removes the ArtCraft logos, badges and links from it.

Rendered icons (PNG/ICO/ICNS) need resvg and Pillow, which this script must not depend on: run
`python ubunye/brand/render_icons.py` after the SVG changes. --check verifies them by hash.
"""
from __future__ import annotations

import argparse
import base64
import json
import os
import re
import shutil
import subprocess
import sys
import tomllib
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent

with open(ROOT / "ubunye" / "brand.toml", "rb") as fh:
    BRAND = tomllib.load(fh)

# The product name: set ONLY in ubunye/brand.toml ([product] name). Everything below derives from it.
NAME = BRAND["product"]["name"]
DISTRIBUTOR = BRAND["product"]["distributor"]
SHORT = BRAND["product"]["short_name"]
SUITE = BRAND["product"]["suite"]
VENDOR = BRAND["product"]["vendor"]
LICENSE = BRAND["product"]["license"]
BASED_ON = BRAND["product"]["based_on"]
APP_ID = BRAND["identifiers"]["app_id"]
UPGRADE_CODE = BRAND["identifiers"]["windows_upgrade_code"]
HOMEPAGE = BRAND["links"]["homepage"]
SUITE_URL = BRAND["links"]["suite"]
DOWNLOADS = BRAND["links"]["downloads"]
SOURCE = BRAND["links"]["source"]
ISSUES = BRAND["links"]["issues"]
UPSTREAM = BRAND["links"]["upstream"]
TAGLINE = f"{NAME}, part of {SUITE}."
OLD_APP_ID = BRAND["upstream_identity_to_replace"]["app_id"]
OLD_VENDOR = BRAND["upstream_identity_to_replace"]["vendor"]

# Our own component GUID for the MSI's main component (upstream's would make Windows Installer treat
# the two products' files as one shared component). Derived, so it never changes.
COMPONENT_GUID = str(uuid.uuid5(uuid.NAMESPACE_DNS, APP_ID + ".msi.component.app")).upper()

ICON_SVG = ROOT / "ubunye" / "brand" / "assets" / "icon.svg"
ICON_DIR = ROOT / "assets" / "app-icon"
ICON_MANIFEST = HERE / "icons.sha256"

PRODUCT_RE = re.compile(r"LightCraft(?![A-Za-z0-9_])")

# ---------------------------------------------------------------------------------------------
# Allow-list: paths where upstream names, links and marks legitimately stay.
# ---------------------------------------------------------------------------------------------
ALLOW_PREFIXES = (
    "docs/",  # upstream user docs and history (incl. docs/brand/LICENSE-brand.txt)
    "ubunye/",  # our own layer
    "contributors/",  # upstream contributor credits
    ".github/workflows/ubunye-",  # our sync/CI workflows name the upstream on purpose
)
ALLOW_FILES = {
    "LICENSE-MIT",
    "LICENSE-APACHE",
    "NOTICE",
    "UPSTREAM.md",
    "UPSTREAM_STATE.json",
    "AGENTS.md",  # upstream agent instructions (history of the project rules)
    "CLAUDE.md",
    "ROADMAP.md",
    "CHANGELOG.md",
    "flake.lock",
    "crates/engine/src/brand.rs",  # generated: carries the upstream credit link
}
TEXT_EXT = {
    ".rs", ".toml", ".md", ".txt", ".json", ".yml", ".yaml", ".sh", ".ps1", ".xml", ".in", ".wxs",
    ".desktop", ".html", ".js", ".nix", ".plist", ".svg", ".py", ".css", "",
}


def allowed(rel: str) -> bool:
    if rel in ALLOW_FILES or rel.startswith(ALLOW_PREFIXES):
        return True
    base = rel.rsplit("/", 1)[-1]
    return base.startswith(("LICENSE", "ATTRIBUTION", "CHANGELOG"))


def tracked_files() -> list[str]:
    try:
        out = subprocess.run(
            ["git", "-C", str(ROOT), "ls-files", "-z", "--cached", "--others", "--exclude-standard"],
            check=True, capture_output=True,
        ).stdout.decode("utf-8")
        files = [f for f in out.split("\0") if f]
    except (OSError, subprocess.CalledProcessError):
        files = []
        for dirpath, dirnames, filenames in os.walk(ROOT):
            dirnames[:] = [d for d in dirnames if d not in {".git", "target", "node_modules"}]
            for fn in filenames:
                files.append(Path(dirpath, fn).relative_to(ROOT).as_posix())
    return sorted({f for f in files if (ROOT / f).is_file()})


def eol_of(rel: str) -> str:
    """The file's line ending on disk (a Windows checkout with core.autocrlf has CRLF)."""
    try:
        return "\r\n" if b"\r\n" in (ROOT / rel).read_bytes()[:65536] else "\n"
    except OSError:
        return "\n"


def read(rel: str) -> str | None:
    """The file as text with LF line endings (every rule is written with LF)."""
    raw = read_raw(rel)
    return None if raw is None else raw.replace("\r\n", "\n")


def read_raw(rel: str) -> str | None:
    p = ROOT / rel
    try:
        data = p.read_bytes()
    except OSError:
        return None
    if b"\0" in data[:4096]:
        return None
    try:
        return data.decode("utf-8")
    except UnicodeDecodeError:
        return None


class Changes:
    def __init__(self, check: bool):
        self.check = check
        self.changed: list[str] = []
        self.problems: list[str] = []
        self.drift: list[str] = []
        self.pending: dict[str, str] = {}

    def get(self, rel: str) -> str | None:
        if rel in self.pending:
            return self.pending[rel]
        return read(rel)

    def put(self, rel: str, text: str) -> None:
        old = read(rel)
        if old == text:
            self.pending.pop(rel, None)
            return
        self.pending[rel] = text
        if rel not in self.changed:
            self.changed.append(rel)

    def flush(self) -> None:
        if self.check:
            return
        for rel, text in self.pending.items():
            p = ROOT / rel
            eol = eol_of(rel)
            p.parent.mkdir(parents=True, exist_ok=True)
            with open(p, "w", encoding="utf-8", newline="") as fh:
                fh.write(text.replace("\n", eol) if eol != "\n" else text)
        self.pending.clear()


# ---------------------------------------------------------------------------------------------
# 1. crates/engine/src/brand.rs
# ---------------------------------------------------------------------------------------------
def rust_str(s: str) -> str:
    return json.dumps(s, ensure_ascii=False)


def brand_rs() -> str:
    consts = [
        ("NAME", NAME, "The product name, everywhere the UI names the app."),
        ("SHORT_NAME", SHORT, "Short form, where upstream used one."),
        ("SUITE", SUITE, "The suite this app belongs to."),
        ("VENDOR", VENDOR, "Vendor / manufacturer / maintainer."),
        ("DISTRIBUTOR", DISTRIBUTOR, "Who distributes the builds."),
        ("TAGLINE", TAGLINE, "One line for About: the product and its suite."),
        ("LICENSE", LICENSE, "Licence of this distribution (same as upstream)."),
        ("APP_ID", APP_ID, "Reverse-DNS app id: Wayland app id, .desktop file, hicolor icon, bundle id, config folder."),
        ("HOMEPAGE", HOMEPAGE, "Product page."),
        ("SUITE_URL", SUITE_URL, "Suite page."),
        ("SOURCE", SOURCE, "Source repository."),
        ("ISSUES", ISSUES, "Issue tracker."),
        ("BASED_ON", BASED_ON, "Plain-text upstream credit (no ArtCraft marks)."),
        ("UPSTREAM", UPSTREAM, "The upstream project the credit links to."),
    ]
    out = [
        "// @generated by ubunye/brand/apply.py from ubunye/brand.toml. Do not edit by hand:",
        "// change ubunye/brand.toml and rerun `python -I ubunye/brand/apply.py`.",
        "// Ubunye AI Ecosystems, 2026. MIT OR Apache-2.0.",
        "",
        "//! The Ubunye brand of this app: product name, ids and links in one place.",
        "",
    ]
    for name, value, doc in consts:
        out.append(f"/// {doc}")
        out.append(f"pub const {name}: &str = {rust_str(value)};")
    return "\n".join(out) + "\n"


# ---------------------------------------------------------------------------------------------
# 2. Exact rules: (path, upstream text, our text). Written against upstream text.
# ---------------------------------------------------------------------------------------------
B = "lightcraft_engine::brand"
RULES: list[tuple[str, str, str]] = [
    # brand module
    ("crates/engine/src/lib.rs", "pub mod availability;\n",
     "pub mod availability;\npub mod brand; // Ubunye brand layer, generated from ubunye/brand.toml\n"),
    # config folder: derived from the app id, never shared with an installed upstream copy
    ("crates/engine/src/config.rs", '.join("Library/Application Support/LightCraft"))',
     '.join("Library/Application Support").join(crate::brand::APP_ID))'),
    ("crates/engine/src/config.rs", 'PathBuf::from(a).join("LightCraft"))', "PathBuf::from(a).join(crate::brand::APP_ID))"),
    ("crates/engine/src/config.rs", '.map(|c| c.join("lightcraft"))', ".map(|c| c.join(crate::brand::APP_ID))"),
    ("apps/lightcraft-cli/tests/cli.rs", 'let app_config = config.join("lightcraft");',
     f"let app_config = config.join({B}::APP_ID);"),
    # desktop app: app id, --version, --help
    ("apps/lightcraft/src/main.rs", 'const APP_ID: &str = "ai.storyteller.lightcraft";', f"const APP_ID: &str = {B}::APP_ID;"),
    ("apps/lightcraft/src/main.rs", 'println!("lightcraft {}", env!("CARGO_PKG_VERSION"));',
     f'println!("lightcraft {{}} ({{}})", env!("CARGO_PKG_VERSION"), {B}::NAME);'),
    ("apps/lightcraft/src/main.rs", "lightcraft — photo library + raw developer\n",
     f"lightcraft — {NAME}: photo library + raw developer ({BASED_ON})\n"),
    ("apps/lightcraft-cli/src/main.rs", 'println!("lightcraft-cli {}", env!("CARGO_PKG_VERSION"));',
     f'println!("lightcraft-cli {{}} ({{}})", env!("CARGO_PKG_VERSION"), {B}::NAME);'),
    ("apps/lightcraft-cli/src/main.rs", "lightcraft-cli — headless LightCraft (photo library + raw developer)\n",
     f"lightcraft-cli — headless {NAME} (photo library + raw developer; {BASED_ON})\n"),
    # Windows VERSIONINFO
    ("apps/lightcraft/build.rs", 'res.set_icon("../../assets/app-icon/lightcraft.ico")\n',
     f'res.set_icon("../../assets/app-icon/lightcraft.ico")\n        .set("CompanyName", "{VENDOR}")\n'),
    ("apps/lightcraft/build.rs", '"Copyright (c) the LightCraft authors. MIT OR Apache-2.0."',
     f'"Copyright (c) the LightCraft authors and {VENDOR}. MIT OR Apache-2.0."'),
    ("apps/lightcraft/build.rs", "show the lynx.", "show the app icon."),
    # network identity
    ("crates/fetch/src/http.rs", "User-Agent: LightCraft/{}", "User-Agent: " + NAME.replace(" ", "") + "/{}"),
    # links (Help menu, About, top bar)
    ("crates/ui-egui/src/links.rs", "//! Community and project links (Help menu, About dialog, top-bar Discord button).",
     "//! Community and project links (Help menu, About dialog, top-bar questions button)."),
    ("crates/ui-egui/src/links.rs", '/// The app\'s id in ArtCraft URLs.\npub const APP: &str = "lightcraft";',
     '/// The app\'s id in product-page URLs.\npub const APP: &str = "light";'),
    ("crates/ui-egui/src/links.rs", "/// The ArtCraft community Discord.\npub const DISCORD: &str = \"https://discord.gg/artcraft\";",
     f"/// Questions and ideas (the issue tracker; the command id stays `app.discord`).\npub const DISCORD: &str = {B}::ISSUES;"),
    ("crates/ui-egui/src/links.rs", '/// The ArtCraft website.\npub const WEBSITE: &str = "https://getartcraft.com";',
     f"/// The suite's website.\npub const WEBSITE: &str = {B}::SUITE_URL;"),
    ("crates/ui-egui/src/links.rs", '/// This app\'s page on the ArtCraft website.\npub const APP_PAGE: &str = "https://getartcraft.com/apps/lightcraft";',
     f"/// This app's product page.\npub const APP_PAGE: &str = {B}::HOMEPAGE;"),
    ("crates/ui-egui/src/links.rs", 'pub const GITHUB: &str = "https://github.com/storytold/lightcraft";',
     f"pub const GITHUB: &str = {B}::SOURCE;"),
    ("crates/ui-egui/src/links.rs", 'assert!(APP_PAGE.ends_with(&format!("/apps/{APP}")));',
     'assert!(APP_PAGE.ends_with(&format!("/tools/{APP}")));'),
    ("crates/ui-egui/src/links.rs", 'assert!(GITHUB.ends_with(&format!("/storytold/{APP}")));',
     'assert!(GITHUB.ends_with("/ubunye-ai-ecosystems/ubunye-creative-light"));'),
    ("crates/ui-egui/src/icons.rs", "/// Community chat (opens the ArtCraft Discord): a speech bubble",
     "/// Community chat (opens the questions and ideas page): a speech bubble"),
    # About: the plain-text upstream credit, linking to the upstream repository
    ("crates/ui-egui/src/panels/dialogs.rs",
     "                            ui.add_space(10.0);\n                            let discord = egui::Button::new(",
     "                            ui.label(" + B + "::TAGLINE);\n"
     "                            let credit = ui.link(" + B + "::BASED_ON).on_hover_text(" + B + "::UPSTREAM);\n"
     "                            if credit.clicked() {\n"
     "                                let _ = crate::links::open(app, " + B + "::UPSTREAM);\n"
     "                            }\n"
     "                            ui.add_space(10.0);\n                            let discord = egui::Button::new("),
    # packaging: Windows
    ("packaging/windows/lightcraft.wxs", 'UpgradeCode="229BB552-5EFD-479A-BA86-8CB6D926390C"', f'UpgradeCode="{UPGRADE_CODE}"'),
    ("packaging/windows/lightcraft.wxs", 'Guid="8A1D3A39-F7C1-47B1-97AE-40C80BF82F4A"', f'Guid="{COMPONENT_GUID}"'),
    ("packaging/windows/lightcraft.wxs", 'Key="Software\\LightCraft"', f'Key="Software\\{VENDOR}\\{NAME}"'),
    ("packaging/windows/lightcraft.wxs", '<Property Id="ARPHELPLINK" Value="https://discord.gg/artcraft" />',
     f'<Property Id="ARPHELPLINK" Value="{ISSUES}" />'),
    ("packaging/windows/lightcraft.wxs", '<Property Id="ARPURLUPDATEINFO" Value="https://getartcraft.com/apps/lightcraft" />',
     f'<Property Id="ARPURLUPDATEINFO" Value="{DOWNLOADS}" />'),
    ("packaging/windows/sign.ps1", "'/d', 'LightCraft', '/du', 'https://github.com/storytold/lightcraft'",
     f"'/d', '{NAME}', '/du', '{HOMEPAGE}'"),
    # packaging: macOS (executable and icon file names stay LightCraft for now)
    ("packaging/macos/Info.plist.in", "<key>CFBundleDisplayName</key>\n  <string>LightCraft</string>",
     f"<key>CFBundleDisplayName</key>\n  <string>{NAME}</string>"),
    ("packaging/macos/Info.plist.in", "<key>CFBundleName</key>\n  <string>LightCraft</string>",
     f"<key>CFBundleName</key>\n  <string>{NAME}</string>"),
    ("packaging/macos/Info.plist.in", "<string>Copyright © the LightCraft authors. MIT OR Apache-2.0.</string>",
     f"<string>Copyright © the LightCraft authors and {VENDOR}. MIT OR Apache-2.0.</string>"),
    ("packaging/macos/Info.plist.in", "<string>LightCraft Document</string>", f"<string>{NAME} Document</string>"),
    ("packaging/macos/package.sh", 'APP="$WORK/LightCraft.app"', f'APP="$WORK/{NAME}.app"'),
    ("packaging/macos/package.sh", 'ditto "$APP" "$STAGE/LightCraft.app"', f'ditto "$APP" "$STAGE/{NAME}.app"'),
    ("packaging/macos/package.sh", '-hfs-volume-name "LightCraft"', f'-hfs-volume-name "{NAME}"'),
    # packaging: Linux
    ("packaging/linux/package.sh",
     'export LIGHTCRAFT_MAINTAINER="${LIGHTCRAFT_MAINTAINER:-LightCraft maintainers <lightcraft@storyteller.ai>}"',
     f'export LIGHTCRAFT_MAINTAINER="${{LIGHTCRAFT_MAINTAINER:-{VENDOR}}}"'),
    ("packaging/linux/package.sh",
     'UPDATE_INFO="gh-releases-zsync|${REPO%%/*}|${REPO#*/}|latest|lightcraft-*-linux-$ARCH.AppImage.zsync"',
     f'UPDATE_INFO="zsync|{DOWNLOADS}{BRAND["product"]["slug"]}-linux-$ARCH.AppImage.zsync"'),
    ("packaging/linux/package.sh", '  REPO="${GITHUB_REPOSITORY:-storytold/lightcraft}"\n', ""),
    ("packaging/linux/package.sh",
     "  # each AppImage on GitHub Releases. `latest` is the newest published release that is not a\n"
     "  # pre-release. A fork's builds point at its own releases through GITHUB_REPOSITORY.\n",
     "  # each AppImage on the Ubunye download server (stable channel). Ubunye: never upstream's releases;\n"
     "  # the release job publishes the .zsync under the fixed name below.\n"),
    (".github/workflows/release.yml", "--appimage-updateinformation | grep -F 'gh-releases-zsync|'",
     f"--appimage-updateinformation | grep -F 'zsync|{DOWNLOADS}'"),
    ("packaging/linux/" + APP_ID + ".metainfo.xml.in", '<developer id="ai.storyteller">', '<developer id="org.ubunye">'),
    ("packaging/linux/nfpm.yaml", "  Lightroom users expect.", "  Lightroom users expect. " + BASED_ON + "."),
]

# Whole-word "LightCraft" -> NAME in these non-Rust files (user-visible names only live there).
WORD_FILES = [
    "packaging/linux/" + APP_ID + ".desktop",
    "packaging/linux/" + APP_ID + ".metainfo.xml.in",
    "packaging/linux/nfpm.yaml",
    "packaging/windows/lightcraft.wxs",
    "apps/lightcraft-web/index.html",
    "nix/package.nix",
    "flake.nix",
]

# Global token replacements (outside the allow-list). Longest first.
TOKENS: list[tuple[str, str]] = [
    ("https://raw.githubusercontent.com/storytold/lightcraft/", SOURCE.replace("https://github.com/", "https://raw.githubusercontent.com/") + "/"),
    ("https://github.com/storytold/lightcraft/issues", ISSUES),
    ("https://github.com/storytold/lightcraft", SOURCE),
    ("https://getartcraft.com/apps/lightcraft", HOMEPAGE),
    ("https://getartcraft.com/apps", SUITE_URL),
    ("https://getartcraft.com/", SUITE_URL + "/"),
    ("https://getartcraft.com", SUITE_URL),
    ("https://discord.gg/artcraft", ISSUES),
    ("repos=storytold/lightcraft", "repos=" + SOURCE.removeprefix("https://github.com/")),
    ("github:storytold/lightcraft", "github:" + SOURCE.removeprefix("https://github.com/")),
    (OLD_APP_ID, APP_ID),
    (OLD_VENDOR, VENDOR),
]

# Upstream labels about ArtCraft/Discord: renamed in Rust literals and in every catalog (key and
# value; the value comes from ubunye/i18n/overlay.json).
RENAMES = {
    "Join the ArtCraft Discord…": "Questions and Ideas…",
    "Join the ArtCraft Discord": "Ask a Question or Share an Idea",
    "Join the ArtCraft community on Discord": "Questions and ideas",
    "ArtCraft Website": SUITE + " Website",
    "ArtCraft — more creative apps": SUITE + ": more creative apps",
}

# Rust literals that keep "LightCraft": metadata identities written into users' files (compatibility
# with existing sidecars and test fixtures), copyright lines and the upstream credit.
KEEP_LINE = [re.compile(p) for p in (
    r"software\s*:\s*Some\(", r"\.software\s*=", r"xmptk", r"\bmake\s*:\s*Some\(", r"UNIQUE_CAMERA_MODEL",
    r"t::SOFTWARE", r'creator\s*:\s*"LightCraft Demo"',
)]

RUST_ROOTS = ("apps/", "crates/")


# ---------------------------------------------------------------------------------------------
# Rust lexer: finds string literals, skipping comments, chars and lifetimes.
# ---------------------------------------------------------------------------------------------
CHAR_RE = re.compile(r"'(?:\\(?:x[0-9a-fA-F]{2}|u\{[0-9a-fA-F]{1,6}\}|.)|[^\\'\n])'")
RAW_RE = re.compile(r'b?r(#*)"')
IDENT_RE = re.compile(r"[A-Za-z0-9_]+")


def rust_literals(text: str):
    """Yield (start, end) of each string literal's content (between the quotes)."""
    i, n = 0, len(text)
    while i < n:
        c = text[i]
        if text.startswith("//", i):
            j = text.find("\n", i)
            i = n if j < 0 else j
        elif text.startswith("/*", i):
            depth, i = 1, i + 2
            while i < n and depth:
                if text.startswith("/*", i):
                    depth, i = depth + 1, i + 2
                elif text.startswith("*/", i):
                    depth, i = depth - 1, i + 2
                else:
                    i += 1
        elif c == '"':
            j = i + 1
            while j < n and text[j] != '"':
                j += 2 if text[j] == "\\" else 1
            yield i + 1, j
            i = j + 1
        elif c == "'":
            m = CHAR_RE.match(text, i)
            i = m.end() if m else i + 1
        elif c.isalpha() or c == "_":
            prev = text[i - 1] if i else " "
            m = RAW_RE.match(text, i) if not (prev.isalnum() or prev == "_") else None
            if m:
                close = '"' + m.group(1)
                start = m.end()
                end = text.find(close, start)
                end = n if end < 0 else end
                yield start, end
                i = end + len(close)
            elif text.startswith("b'", i):
                m2 = CHAR_RE.match(text, i + 1)
                i = m2.end() if m2 else i + 2
            elif text.startswith('b"', i):
                i += 1
            else:
                i = IDENT_RE.match(text, i).end()
        else:
            i += 1


def line_at(text: str, pos: int) -> str:
    start = text.rfind("\n", 0, pos) + 1
    end = text.find("\n", pos)
    return text[start:] if end < 0 else text[start:end]


# Phrases that keep "LightCraft": the upstream credit and copyright/contributor credits.
PROTECTED = re.compile(r"Based on LightCraft by the ArtCraft team|the LightCraft (?:authors|contributors)|LightCraft contributors")


ARTICLE_RE = re.compile(r"\b([Aa]) (?=LightCraft(?![A-Za-z0-9_]))")


def _sub(text: str, article: bool = True) -> str:
    if article and NAME[:1].upper() in "AEIOU":  # "a LightCraft library" -> "an Ubunye Light library"
        text = ARTICLE_RE.sub(lambda m: m.group(1) + "n ", text)
    return PRODUCT_RE.sub(NAME, text)


def sub_product(text: str, article: bool = True) -> str:
    """Replace the product name, leaving the protected credit phrases alone."""
    out, last = [], 0
    for m in PROTECTED.finditer(text):
        out.append(_sub(text[last:m.start()], article))
        out.append(m.group(0))
        last = m.end()
    out.append(_sub(text[last:], article))
    return "".join(out)


def product_leak(text: str) -> bool:
    return bool(PRODUCT_RE.search(PROTECTED.sub("", text)))


def rebrand_literal(content: str, line: str) -> str:
    if content in RENAMES:
        return RENAMES[content]
    if "LightCraft" not in content or any(p.search(line) for p in KEEP_LINE):
        return content
    return sub_product(content)


def rebrand_rust(text: str) -> tuple[str, list[str]]:
    out, last, leaks = [], 0, []
    for s, e in rust_literals(text):
        content = text[s:e]
        new = rebrand_literal(content, line_at(text, s))
        if new != content:
            out.append(text[last:s])
            out.append(new)
            last = e
    out.append(text[last:])
    return "".join(out), leaks


def rust_leaks(text: str) -> list[str]:
    found = []
    for s, e in rust_literals(text):
        content = text[s:e]
        if content in RENAMES or rebrand_literal(content, line_at(text, s)) != content:
            found.append(content[:80])
    return found


# ---------------------------------------------------------------------------------------------
# Locale catalogs
# ---------------------------------------------------------------------------------------------
ENTRY_RE = re.compile(r'^(\s*)("(?:[^"\\]|\\.)*")(\s*:\s*)("(?:[^"\\]|\\.)*")(,?)(\s*)$')


def load_overlay() -> dict[str, dict[str, str]]:
    with open(ROOT / "ubunye" / "i18n" / "overlay.json", encoding="utf-8") as fh:
        raw = json.load(fh)
    fill = lambda t: t.replace("{suite}", SUITE)
    return {lang: {fill(k): fill(v) for k, v in entries.items()} for lang, entries in raw.items() if not lang.startswith("_")}


def rebrand_catalog(rel: str, text: str, overlay: dict[str, dict[str, str]]) -> str:
    lang = Path(rel).name.removesuffix(".json").removesuffix("-formats")
    lines = text.split("\n")
    for idx, line in enumerate(lines):
        m = ENTRY_RE.match(line)
        if not m:
            continue
        key, value = json.loads(m.group(2)), json.loads(m.group(4))
        if key in RENAMES:
            new_key = RENAMES[key]
            new_value = overlay.get(lang, {}).get(new_key)
            if new_value is None:
                raise SystemExit(f"ubunye/i18n/overlay.json: no {lang} translation for {new_key!r}")
            key, value = new_key, new_value
        else:
            key, value = sub_product(key), sub_product(value, article=False)  # English keys only
        enc = lambda s: json.dumps(s, ensure_ascii=False)
        lines[idx] = f"{m.group(1)}{enc(key)}{m.group(3)}{enc(value)}{m.group(5)}{m.group(6)}"
    return "\n".join(lines)


# ---------------------------------------------------------------------------------------------
# README
# ---------------------------------------------------------------------------------------------
HEADER_START = "<!-- ubunye:header (generated by ubunye/brand/apply.py) -->"
HEADER_END = "<!-- /ubunye:header -->"


def readme_header() -> str:
    return "\n".join([
        HEADER_START,
        f"<h1 align=\"center\">{NAME}</h1>",
        "",
        f"<p align=\"center\"><b>{NAME}</b> is the photo library and raw developer of "
        f"<a href=\"{SUITE_URL}\">{SUITE}</a>, by {VENDOR}, distributed by {DISTRIBUTOR}.</p>",
        "",
        f"<p align=\"center\"><a href=\"{HOMEPAGE}\">Product page</a> · <a href=\"{SOURCE}\">Source</a> · "
        f"<a href=\"{ISSUES}\">Issues</a></p>",
        "",
        f"> {BASED_ON} (<a href=\"{UPSTREAM}\">{UPSTREAM}</a>), used under {LICENSE}. "
        "This distribution is not made or endorsed by the ArtCraft team and carries none of the ArtCraft marks. "
        "The rest of this README is upstream's text, adapted to this distribution's name and links.",
        HEADER_END,
    ])


P_BLOCK_RE = re.compile(r'<p align="center">\n(?:.*\n)*?</p>\n\n?', re.MULTILINE)
CRAFTING_RE = re.compile(r"## The Crafting Apps\n.*?(?=## License and credits)", re.DOTALL)
NOTE_RE = re.compile(r"> \[!NOTE\]\n> \*\*ArtCraft is a community.*?\n(?:> .*\n)*\n?")
BRAND_PARA_RE = re.compile(r"The ArtCraft name, wordmark and logos in \[`docs/brand/`\].*?Forks and modified versions must remove them\.\n",
                           re.DOTALL)
DISCORD_HDR_RE = re.compile(r'<h3 align="center">Come make things with us</h3>\n\n')


def rebrand_readme(text: str) -> str:
    # our header, regenerated
    if HEADER_START in text:
        text = re.sub(re.escape(HEADER_START) + r".*?" + re.escape(HEADER_END) + r"\n*", "", text, flags=re.DOTALL)
    # ArtCraft logos, badges and link bars
    text = P_BLOCK_RE.sub(lambda m: "" if re.search(r"artcraft|discord", m.group(0), re.I) else m.group(0), text)
    text = NOTE_RE.sub("", text)
    text = DISCORD_HDR_RE.sub("", text)
    text = CRAFTING_RE.sub(
        f"## {SUITE}\n\n{NAME} is part of [{SUITE}]({SUITE_URL}) by {VENDOR}, distributed by {DISTRIBUTOR}, "
        f"alongside the suite's photo and film apps. {BASED_ON}: LightCraft is one of the Crafting Apps, free and open-source creative "
        f"tools written from scratch in Rust ([upstream repository]({UPSTREAM})).\n\n<br>\n\n",
        text,
    )
    text = BRAND_PARA_RE.sub(
        "This distribution does not include the ArtCraft name, wordmark or logos (their terms are in\n"
        "[`docs/brand/LICENSE-brand.txt`](docs/brand/LICENSE-brand.txt)).\n",
        text,
    )
    text = text.replace('<h1 align="center">LightCraft</h1>\n\n', "")
    text = text.replace("Bring them to [Discord](https://discord.gg/artcraft).", f"Open an [issue]({ISSUES}).")
    text = text.replace("[LightCraft page on getartcraft.com](https://getartcraft.com/apps/lightcraft)",
                        f"[{NAME} product page]({HOMEPAGE})")
    text = text.replace("[latest release](https://github.com/storytold/lightcraft/releases/latest)",
                        f"[download page]({DOWNLOADS})")
    for old, new in TOKENS:
        text = text.replace(old, new)
    # product name where it describes this distribution (not copyright or contributor credits)
    out = []
    for line in text.split("\n"):
        if not re.search(r"Copyright|Crafting Apps", line):
            line = sub_product(line)
        out.append(line)
    text = "\n".join(out).lstrip("\n")
    return readme_header() + "\n\n" + text


# ---------------------------------------------------------------------------------------------
# Static files we own outright
# ---------------------------------------------------------------------------------------------
ICON_LICENSE = f"""{NAME} app icon (lightcraft.svg and every file derived from it in this directory:
lightcraft-small.svg, lightcraft-1024.png, lightcraft-macos-512.png, lightcraft.icns,
lightcraft.ico, hicolor/).

{VENDOR} artwork from the Ubunye Creative Studio brand system, MIT OR Apache-2.0; letter outlines
from DM Serif Display (SIL OFL 1.1). It replaces the upstream LightCraft icon in this distribution.
Source: ubunye/brand/assets/icon.svg (icon-small.svg at 32 px and below). Licensed under either of

  * Apache License, Version 2.0 (https://www.apache.org/licenses/LICENSE-2.0)
  * MIT license (https://opensource.org/licenses/MIT)

at your option.

Regenerate the derived files with `python ubunye/brand/render_icons.py` (or packaging/icons.sh).
"""

ICON_README = f"""# {NAME} app icon

The {NAME} icon: {VENDOR} artwork from the Ubunye Creative Studio brand system, with letter outlines from
DM Serif Display (SIL OFL 1.1). It replaces upstream LightCraft's icon in this distribution.

The source is `ubunye/brand/assets/icon.svg` (a full-bleed 512 tile, `viewBox="0 0 512 512"`).
Every file here is generated from it; do not edit them by hand. macOS files (`.icns`,
`lightcraft-macos-512.png`) add Apple's transparent margin (an 824/1024 body); Windows and Linux files
use the full-bleed tile. The file names `lightcraft*` are kept from upstream so packaging scripts and
`include_bytes!` paths stay unchanged; the hicolor names follow the app id `{APP_ID}`.

| File | What |
|---|---|
| `lightcraft.svg`, `lightcraft-small.svg` | copies of the source SVG |
| `lightcraft-1024.png` | 1024 px render |
| `lightcraft-macos-512.png` | runtime window/Dock icon on macOS (embedded by `apps/lightcraft/src/main.rs`) |
| `lightcraft.icns` | macOS bundle icon |
| `lightcraft.ico` | Windows icon, 16-256 px, embedded in `lightcraft.exe` by `apps/lightcraft/build.rs` |
| `hicolor/<n>x<n>/apps/{APP_ID}.png` | Linux icon theme, 16-512 px; the 256 px one is also the runtime window icon on Windows and Linux |
| `hicolor/scalable/apps/{APP_ID}.svg` | Linux scalable icon |

To change the icon, replace `ubunye/brand/assets/icon.svg` and run `python -I ubunye/brand/apply.py`
(it rerenders through `ubunye/brand/render_icons.py`, which needs resvg and Pillow). Licence: `LICENSE.txt`.
"""

ATTRIBUTION_MARK = "## Ubunye distribution (appended by Ubunye AI Ecosystems)"
ATTRIBUTION_APPEND = f"""
{ATTRIBUTION_MARK}

In this distribution the upstream app icon (the `assets/app-icon/` row above) and the ArtCraft marks
(the `docs/brand/` row above) are not used: the icon files are replaced and the logo files removed.

| Path | Asset | Creator | Source | Licence | Added | Modifications |
|---|---|---|---|---|---|---|
| `ubunye/brand/assets/` (`icon.svg`, `icon-small.svg`), copied to `assets/app-icon/` with the PNG/ICNS/ICO files rendered from it | {NAME} app icon | {VENDOR} | Ubunye Creative Studio brand system (ubunye-creative-studio-tools/branding); letter outlines from DM Serif Display | MIT OR Apache-2.0; letter outline SIL Open Font License 1.1 | 2026-10-11 | rendered to PNG/ICO/ICNS by `ubunye/brand/render_icons.py` |
"""


# ---------------------------------------------------------------------------------------------
# Icons and file moves
# ---------------------------------------------------------------------------------------------
def favicon_tag() -> str:
    data = base64.b64encode(ICON_SVG.read_bytes().replace(b"\r\n", b"\n")).decode("ascii")
    return f'<link rel="icon" type="image/svg+xml" href="data:image/svg+xml;base64,{data}">'


def moves(ch: Changes) -> None:
    pairs = []
    for rel in tracked_files():
        if OLD_APP_ID in rel.rsplit("/", 1)[-1] and rel.startswith(("packaging/", "assets/app-icon/")):
            pairs.append((rel, rel.replace(OLD_APP_ID, APP_ID)))
    for old, new in pairs:
        ch.changed.append(f"{old} -> {new}")
        if ch.check:
            continue
        (ROOT / new).parent.mkdir(parents=True, exist_ok=True)
        if (ROOT / new).exists():
            (ROOT / new).unlink()
        shutil.move(ROOT / old, ROOT / new)


def icons(ch: Changes) -> None:
    svg = ICON_SVG.read_text(encoding="utf-8")
    small_src = ICON_SVG.with_name("icon-small.svg")
    small = small_src.read_text(encoding="utf-8") if small_src.exists() else svg
    for rel in ("assets/app-icon/lightcraft.svg", f"assets/app-icon/hicolor/scalable/apps/{APP_ID}.svg"):
        ch.put(rel, svg)
    ch.put("assets/app-icon/lightcraft-small.svg", small)
    ch.put("assets/app-icon/LICENSE.txt", ICON_LICENSE)
    for p in sorted((ROOT / "docs" / "brand").glob("artcraft-*")):
        ch.changed.append(f"delete {p.relative_to(ROOT).as_posix()}")
        if not ch.check:
            p.unlink()


def file_digest(p: Path) -> str:
    """sha256 of a file; text (SVG) is hashed with LF endings so Windows and Linux checkouts agree."""
    import hashlib

    data = p.read_bytes()
    if p.suffix.lower() == ".svg":
        data = data.replace(b"\r\n", b"\n")
    return hashlib.sha256(data).hexdigest()


def icons_current() -> bool:
    """True when icons.sha256 was rendered from the current icon.svg."""
    import hashlib

    if not ICON_MANIFEST.exists():
        return False
    want = file_digest(ICON_SVG) + "  ubunye/brand/assets/icon.svg"
    if want not in ICON_MANIFEST.read_text(encoding="utf-8").splitlines():
        return False
    probe = Changes(check=True)
    check_icon_hashes(probe)
    return not probe.problems


def render_icons_if_needed() -> None:
    """Rerender PNG/ICO/ICNS when icon.svg changed. Rendering needs resvg and Pillow, so it runs
    in a separate, non-isolated interpreter; without them it only warns (and --check fails)."""
    if icons_current():
        return
    script = HERE / "render_icons.py"
    env = {k: v for k, v in os.environ.items() if k not in {"PYTHONNOUSERSITE", "PYTHONSAFEPATH"}}
    r = subprocess.run([sys.executable, str(script)], env=env, cwd=str(ROOT))
    if r.returncode:
        print("warning: icons not rendered; run `python ubunye/brand/render_icons.py` (needs resvg and Pillow)")


def check_icon_hashes(ch: Changes) -> None:
    import hashlib

    if not ICON_MANIFEST.exists():
        ch.problems.append("ubunye/brand/icons.sha256 missing: run python ubunye/brand/render_icons.py")
        return
    for line in ICON_MANIFEST.read_text(encoding="utf-8").splitlines():
        if not line.strip():
            continue
        digest, rel = line.split("  ", 1)
        p = ROOT / rel
        if not p.exists() or file_digest(p) != digest:
            ch.problems.append(f"{rel}: not the rendered Ubunye icon (run python ubunye/brand/render_icons.py)")
    for p in (ROOT / "assets" / "app-icon").rglob(f"{OLD_APP_ID}*"):
        ch.problems.append(f"{p.relative_to(ROOT).as_posix()}: upstream icon file name")


# ---------------------------------------------------------------------------------------------
# Main passes
# ---------------------------------------------------------------------------------------------
def apply_rules(ch: Changes) -> None:
    for rel, old, new in RULES:
        text = ch.get(rel)
        if text is None:
            ch.drift.append(f"{rel}: file missing (rule {old[:50]!r})")
            continue
        if new and new in text:
            continue  # applied
        if old in text:
            ch.put(rel, text.replace(old, new))
        elif new:
            ch.drift.append(f"{rel}: upstream text changed, rule no longer matches: {old[:70]!r}")


def apply_tokens_and_words(ch: Changes, files: list[str]) -> None:
    for rel in files:
        if allowed(rel) or rel == "README.md" or Path(rel).suffix.lower() not in TEXT_EXT:
            continue
        text = ch.get(rel)
        if text is None:
            continue
        new = text
        for old, rep in TOKENS:
            new = new.replace(old, rep)
        if rel in WORD_FILES:
            new = sub_product(new)
        if new != text:
            ch.put(rel, new)


def apply_rust_and_catalogs(ch: Changes, files: list[str]) -> None:
    overlay = load_overlay()
    for rel in files:
        if allowed(rel):
            continue
        if rel.endswith(".rs") and rel.startswith(RUST_ROOTS):
            text = ch.get(rel)
            if text and ("LightCraft" in text or "ArtCraft" in text):
                new, _ = rebrand_rust(text)
                if new != text:
                    ch.put(rel, new)
        elif rel.startswith("crates/ui-egui/locales/") and rel.endswith(".json"):
            text = ch.get(rel)
            if text:
                new = rebrand_catalog(rel, text, overlay)
                if new != text:
                    ch.put(rel, new)


def apply_static(ch: Changes) -> None:
    ch.put("crates/engine/src/brand.rs", brand_rs())
    ch.put("assets/app-icon/README.md", ICON_README)
    web = ch.get("apps/lightcraft-web/index.html")
    if web is not None:
        ch.put("apps/lightcraft-web/index.html", re.sub(r'<link rel="icon"[^>]*>', lambda _m: favicon_tag(), web, count=1))
    readme = ch.get("README.md")
    if readme is not None:
        ch.put("README.md", rebrand_readme(readme))
    attribution = ch.get("assets/ATTRIBUTION.md") or ""
    upstream_part = attribution.split("\n" + ATTRIBUTION_MARK, 1)[0]
    ch.put("assets/ATTRIBUTION.md", upstream_part.rstrip("\n") + "\n" + ATTRIBUTION_APPEND)


LEAK_RE = re.compile(r"getartcraft|ai\.storyteller|Learning Machines|discord\.gg/artcraft|storytold/lightcraft|artcraft-(?:logo|mark)")
ARTCRAFT_OK = re.compile(r"Based on LightCraft by the ArtCraft team|Copyright|LICENSE-brand|ArtCraft (?:name|marks)")


def leak_scan(ch: Changes, files: list[str]) -> None:
    for rel in files:
        if allowed(rel):
            continue
        text = ch.get(rel)
        if text is None:
            continue
        for no, line in enumerate(text.split("\n"), 1):
            if BASED_ON in line:
                continue  # the upstream credit may name and link the upstream
            if LEAK_RE.search(line):
                ch.problems.append(f"{rel}:{no}: upstream identity or link: {line.strip()[:100]}")
            elif "ArtCraft" in line and not ARTCRAFT_OK.search(line):
                ch.problems.append(f"{rel}:{no}: ArtCraft outside the credit: {line.strip()[:100]}")
        if rel.endswith(".rs") and rel.startswith(RUST_ROOTS):
            for lit in rust_leaks(text):
                ch.problems.append(f"{rel}: user-visible LightCraft string: {lit!r}")
        elif rel.startswith("crates/ui-egui/locales/") and rel.endswith(".json"):
            for no, line in enumerate(text.split("\n"), 1):
                if product_leak(line):
                    ch.problems.append(f"{rel}:{no}: catalog still says LightCraft")
        elif rel in WORD_FILES:
            for no, line in enumerate(text.split("\n"), 1):
                if product_leak(line):
                    ch.problems.append(f"{rel}:{no}: LightCraft in packaging/web metadata")
    for p in (ROOT / "docs" / "brand").glob("artcraft-*"):
        ch.problems.append(f"{p.relative_to(ROOT).as_posix()}: ArtCraft logo file present")
    for rel in files:
        if rel.startswith(("apps/", "crates/")) and rel.endswith((".rs", ".html")):
            text = ch.get(rel) or ""
            if "docs/brand" in text:
                ch.problems.append(f"{rel}: references docs/brand (ArtCraft marks)")


def format_rust(files: list[str]) -> None:
    """rustfmt the Rust files this run changed: a longer product name can push a line past
    max_width, and rustfmt then lays the expression out differently. Optional (warns without it)."""
    if not files:
        return
    rustfmt = os.environ.get("RUSTFMT") or shutil.which("rustfmt")
    if not rustfmt:
        print("warning: rustfmt not found; run `cargo fmt --all` before committing")
        return
    r = subprocess.run([rustfmt, "--edition", "2024", "--config-path", str(ROOT / "rustfmt.toml"), *files], cwd=str(ROOT))
    if r.returncode:
        print("warning: rustfmt failed; run `cargo fmt --all` before committing")


def main() -> int:
    for stream in (sys.stdout, sys.stderr):
        if hasattr(stream, "reconfigure"):
            stream.reconfigure(encoding="utf-8", errors="replace")
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--check", action="store_true", help="change nothing; exit 1 when anything is not branded")
    args = ap.parse_args()
    ch = Changes(args.check)

    moves(ch)
    files = tracked_files()
    if args.check:
        # Pretend the moves happened, so the scan reads the files under their new names.
        files = [f.replace(OLD_APP_ID, APP_ID) if f.startswith(("packaging/", "assets/app-icon/")) else f for f in files]
        files = [f for f in files if (ROOT / f).exists()]
    apply_rules(ch)
    apply_tokens_and_words(ch, files)
    apply_rust_and_catalogs(ch, files)
    apply_static(ch)
    icons(ch)

    if args.check:
        if ch.changed:
            ch.problems.extend(f"would change: {c}" for c in ch.changed)
        leak_scan(ch, files)
        check_icon_hashes(ch)
        for d in ch.drift:
            print(f"drift: {d}")
        if ch.problems:
            for p in ch.problems:
                print(f"leak: {p}")
            print(f"\n{len(ch.problems)} problem(s). Run: python -I ubunye/brand/apply.py")
            return 1
        print("brand check passed: no leaks, every scripted change is applied" + (f" ({len(ch.drift)} drift note(s))" if ch.drift else ""))
        return 0

    rust_changed = [c for c in ch.pending if c.endswith(".rs")]
    ch.flush()
    format_rust(rust_changed)
    render_icons_if_needed()
    for c in ch.changed:
        print(f"updated: {c}")
    for d in ch.drift:
        print(f"drift: {d}")
    leak_scan(ch, tracked_files())
    check_icon_hashes(ch)
    for p in ch.problems:
        print(f"remaining: {p}")
    print(f"{len(ch.changed)} change(s); {len(ch.problems)} remaining problem(s)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
