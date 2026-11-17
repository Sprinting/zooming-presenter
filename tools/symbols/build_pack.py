#!/usr/bin/env python3
"""Builds Zebra's clip-art pack (Insert > Symbols & Shapes) from the sources
Kartik picked (C1-C36 in the preset catalogue).

    python3 -I tools/symbols/build_pack.py --npm <node_modules> --vendor <dir> \
        --tango <tango 0.8.90 dir> --openclipart <openclipart svg dir> \
        [--people <dicebear output dir>] --out assets/symbols

Nothing here runs code from the sources: Iconify sets are JSON, Humaaans and
Open Doodles are read as text (JSX / createElement calls), Tango and
Openclipart are plain SVG files.

Output, one pair of files per set plus a list of sets:
    sets.json          [{id, key, name, group, licence, ...}]
    <key>.idx          name \t keywords \t offset \t length   (one line per symbol)
    <key>.dat          the SVG documents, back to back (UTF-8)
The app keeps the .idx files in memory and reads single SVGs from .dat on
demand.
"""
import argparse
import json
import os
import re
import sys

# id, Iconify prefix, group, tint
# tint: "mono" = drawn in currentColor, can follow the theme;
#       "two"  = IconPark's outline + fill colours, both follow the theme;
#       ""     = full colour, drawn as is.
ICONIFY = [
    ("C1", "noto", "emoji", ""),
    ("C2", "noto-v1", "emoji", ""),
    ("C3", "fluent-emoji", "emoji", ""),
    ("C4", "fluent-emoji-flat", "emoji", ""),
    ("C5", "twemoji", "emoji", ""),
    ("C7", "fxemoji", "emoji", ""),
    ("C10", "flat-color-icons", "colour", ""),
    ("C11", "icon-park", "colour", "two"),
    ("C12", "fluent-color", "colour", ""),
    ("C13", "streamline-ultimate-color", "colour", ""),
    ("C14", "streamline-kameleon-color", "colour", ""),
    ("C15", "streamline-stickies-color", "colour", ""),
    ("C17", "glyphs-poly", "colour", ""),
    ("C18", "healthicons", "mono", "mono"),
    ("C19", "streamline-freehand", "drawn", "mono"),
    ("C20", "pepicons-pencil", "drawn", "mono"),
    ("C27", "game-icons", "mono", "mono"),
    ("C28", "ph", "mono", "mono"),
    ("C29", "tabler", "mono", "mono"),
    ("C30", "iconoir", "mono", "mono"),
    ("C31", "material-symbols", "mono", "mono"),
    ("C32", "mingcute", "mono", "mono"),
    ("C33", "solar", "mono", "mono"),
    ("C34", "lucide-lab", "mono", "mono"),
    ("C35", "streamline-plump", "mono", "mono"),
]

CREDIT_NEEDED = {"CC-BY-3.0", "CC-BY-4.0"}


class Writer:
    def __init__(self, out, key):
        self.key = key
        self.dat = open(os.path.join(out, key + ".dat"), "wb")
        self.idx = open(os.path.join(out, key + ".idx"), "w", encoding="utf-8")
        self.offset = 0
        self.count = 0
        self.seen = set()

    def add(self, name, keywords, svg):
        name = clean(name)
        if not name or name in self.seen:
            return
        self.seen.add(name)
        b = svg.encode("utf-8")
        self.dat.write(b)
        kw = ",".join(dict.fromkeys(clean(k) for k in keywords if clean(k) and clean(k) != name))
        self.idx.write(f"{name}\t{kw}\t{self.offset}\t{len(b)}\n")
        self.offset += len(b)
        self.count += 1

    def close(self):
        self.dat.close()
        self.idx.close()


def clean(s):
    return re.sub(r"[\t\n\r,]+", " ", str(s)).strip().lower()


def iconify_svg(body, w, h, left=0, top=0):
    return (f'<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" '
            f'viewBox="{left} {top} {w} {h}" width="{w}" height="{h}">{body}</svg>')


def build_iconify(npm, out, sid, prefix, group, tint):
    base = os.path.join(npm, "@iconify-json", prefix)
    data = json.load(open(os.path.join(base, "icons.json"), encoding="utf-8"))
    info = json.load(open(os.path.join(base, "info.json"), encoding="utf-8"))
    cats = {}
    try:
        meta = json.load(open(os.path.join(base, "metadata.json"), encoding="utf-8"))
        for c, names in (meta.get("categories") or {}).items():
            for n in names:
                cats.setdefault(n, []).append(c)
    except FileNotFoundError:
        pass
    try:
        chars = json.load(open(os.path.join(base, "chars.json"), encoding="utf-8"))
    except FileNotFoundError:
        chars = {}
    char_of = {}
    for ch, n in chars.items():
        char_of.setdefault(n, ch)
    aliases = {}
    for a, v in (data.get("aliases") or {}).items():
        if not v.get("hidden") and set(v) <= {"parent"}:
            aliases.setdefault(v["parent"], []).append(a)
    dw, dh = data.get("width", 16), data.get("height", 16)
    dl, dt = data.get("left", 0), data.get("top", 0)
    wr = Writer(out, prefix)
    for name in sorted(data["icons"]):
        icon = data["icons"][name]
        if icon.get("hidden"):
            continue
        svg = iconify_svg(icon["body"], icon.get("width", dw), icon.get("height", dh), icon.get("left", dl), icon.get("top", dt))
        words = name.split("-") + cats.get(name, []) + aliases.get(name, [])
        for a in aliases.get(name, []):
            words += a.split("-")
        wr.add(name, words, svg)
    wr.close()
    lic = info["license"]
    return {
        "id": sid, "key": prefix, "name": info["name"], "group": group, "tint": tint,
        "author": info["author"]["name"], "author_url": info["author"].get("url", ""),
        "licence": lic["title"], "licence_spdx": lic.get("spdx", ""), "licence_url": lic.get("url", ""),
        "credit": lic.get("spdx", "") in CREDIT_NEEDED, "count": wr.count,
    }


# ---- Humaaans (C24): JSX parts composed into whole figures ----

ATTR = {
    "strokeWidth": "stroke-width", "fillRule": "fill-rule", "fillOpacity": "fill-opacity",
    "strokeLinecap": "stroke-linecap", "strokeLinejoin": "stroke-linejoin", "strokeOpacity": "stroke-opacity",
    "stopColor": "stop-color", "stopOpacity": "stop-opacity", "clipRule": "clip-rule", "clipPath": "clip-path",
    "strokeMiterlimit": "stroke-miterlimit", "strokeDasharray": "stroke-dasharray", "xlinkHref": "xlink:href",
    "xmlnsXlink": "xmlns:xlink", "fontFamily": "font-family", "fontSize": "font-size", "fontWeight": "font-weight",
}


def jsx_part(path):
    src = open(path, encoding="utf-8").read()
    body = src[src.index("<"): src.rindex(">") + 1]
    body = re.sub(r"\s*\{\.\.\.props\}", "", body)
    body = re.sub(r'=\{"([^"]*)"\}', r'="\1"', body)
    body = re.sub(r"=\{([^}]*)\}", r'="\1"', body)
    body = re.sub(r"\b([a-zA-Z]+)(?==\")", lambda m: ATTR.get(m.group(1), m.group(1)), body)
    # Ids repeat across parts; nothing references them except masks/uses,
    # which keep working because each part's ids get its own prefix.
    return body


def words_of(camel):
    return re.sub(r"([a-z])([A-Z0-9])", r"\1 \2", camel).lower()


def build_humaaans(vendor, out):
    base = os.path.join(vendor, "humaaans", "src", "human", "body-parts")
    part = lambda kind: sorted(f[:-3] for f in os.listdir(os.path.join(base, kind)) if f.endswith(".js"))
    heads, torsos, standing, sitting = part("head"), part("torso"), part("standing"), part("sitting")
    cache = {}

    def get(kind, name, tag):
        k = (kind, name)
        if k not in cache:
            cache[k] = jsx_part(os.path.join(base, kind, name + ".js"))
        # Give each copy its own ids so url(#...) references stay inside it.
        return re.sub(r'(id="|url\(#|xlink:href="#)([^")]+)', lambda m: m.group(1) + tag + m.group(2), cache[k])

    def figure(head, torso, bottom, posture):
        h = 400 if posture == "sitting" else 480
        adj = 24 if posture == "sitting" else 31
        return (f'<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 380 {h}" width="380" height="{h}">'
                f'<g fill-rule="evenodd" stroke-width="1"><g transform="translate(40, {adj})">'
                f'<g transform="translate(82, 0)">{get("head", head, "h-")}</g>'
                f'<g transform="translate(0, 187)">{get(posture, bottom, "b-")}</g>'
                f'<g transform="translate(22, 82)">{get("torso", torso, "t-")}</g>'
                f'</g></g></svg>')

    wr = Writer(out, "humaaans")
    for i, head in enumerate(heads):
        for j, (posture, bottoms, torso_step) in enumerate([("standing", standing, 0), ("standing", standing, 5), ("sitting", sitting, 3)]):
            torso = torsos[(i + torso_step) % len(torsos)]
            bottom = bottoms[(i + j) % len(bottoms)]
            name = f"{posture} {words_of(head)} {words_of(torso)} {words_of(bottom)}"
            wr.add(name, ["person", "people", "human", posture, words_of(head), words_of(torso), words_of(bottom)], figure(head, torso, bottom, posture))
    wr.close()
    return {
        "id": "C24", "key": "humaaans", "name": "Humaaans", "group": "people", "tint": "",
        "author": "Pablo Stanley", "author_url": "https://www.humaaans.com",
        "licence": "CC0 1.0", "licence_spdx": "CC0-1.0", "licence_url": "https://creativecommons.org/publicdomain/zero/1.0/",
        "credit": False, "count": wr.count,
    }


# ---- Open Doodles (C25): React.createElement calls read as data ----

class CE:
    """Reads `React.createElement("tag", {props}, children...)` without running it."""

    def __init__(self, s, consts):
        self.s, self.i, self.consts = s, 0, consts

    def ws(self):
        while self.i < len(self.s) and self.s[self.i] in " \t\r\n":
            self.i += 1

    def expect(self, t):
        self.ws()
        if not self.s.startswith(t, self.i):
            raise ValueError(f"expected {t!r} at {self.s[self.i:self.i + 40]!r}")
        self.i += len(t)

    def string(self):
        self.ws()
        q = self.s[self.i]
        j = self.i + 1
        out = []
        while self.s[j] != q:
            if self.s[j] == "\\":
                j += 1
            out.append(self.s[j])
            j += 1
        self.i = j + 1
        return "".join(out)

    def value(self):
        self.ws()
        c = self.s[self.i]
        if c in "\"'":
            return self.string()
        m = re.compile(r"[-\w.]+").match(self.s, self.i)
        self.i = m.end()
        v = m.group(0)
        return self.consts.get(v, v)

    def element(self):
        self.expect("React.createElement(")
        tag = self.string()
        self.expect(",")
        self.ws()
        props = {}
        if self.s.startswith("null", self.i):
            self.i += 4
        else:
            self.expect("{")
            while True:
                self.ws()
                if self.s[self.i] == "}":
                    self.i += 1
                    break
                if self.s[self.i] in "\"'":
                    k = self.string()
                else:
                    m = re.compile(r"[\w$]+").match(self.s, self.i)
                    k = m.group(0)
                    self.i = m.end()
                self.expect(":")
                props[k] = self.value()
                self.ws()
                if self.s[self.i] == ",":
                    self.i += 1
        kids = []
        while True:
            self.ws()
            if self.s[self.i] == ")":
                self.i += 1
                break
            self.expect(",")
            self.ws()
            if self.s[self.i] == ")":
                continue
            kids.append(self.element())
        attrs = "".join(f' {ATTR.get(k, k)}="{v}"' for k, v in props.items())
        return f"<{tag}{attrs}>{''.join(kids)}</{tag}>"


def build_opendoodles(vendor, out):
    src = open(os.path.join(vendor, "opendoodles", "dist", "index.es.js"), encoding="utf-8").read()
    consts = {"ink": "#000", "accent": "#FF5678"}
    wr = Writer(out, "open-doodles")
    for m in re.finditer(r"var (\w+)Doodle = function \(_a\) \{.*?return \(", src, re.S):
        p = CE(src, consts)
        p.i = m.end()
        svg = p.element()
        name = words_of(m.group(1))
        wr.add(name, ["person", "people", "doodle", "sketch"] + name.split(), svg)
    wr.close()
    return {
        "id": "C25", "key": "open-doodles", "name": "Open Doodles", "group": "people", "tint": "",
        "author": "Pablo Stanley", "author_url": "https://www.opendoodles.com",
        "licence": "CC0 1.0", "licence_spdx": "CC0-1.0", "licence_url": "https://creativecommons.org/publicdomain/zero/1.0/",
        "credit": False, "count": wr.count,
    }


# ---- Plain SVG trees: Tango (C36) and Openclipart (C26) ----

def slim(svg):
    """Drops editor bookkeeping that never draws anything."""
    svg = re.sub(r"<metadata\b.*?</metadata>", "", svg, flags=re.S)
    svg = re.sub(r"<sodipodi:namedview\b.*?(/>|</sodipodi:namedview>)", "", svg, flags=re.S)
    svg = re.sub(r"<!--.*?-->", "", svg, flags=re.S)
    svg = re.sub(r"<\?xml[^>]*\?>|<!DOCTYPE[^>[]*(\[.*?\])?\s*>", "", svg, flags=re.S)
    return svg.strip()


def build_tree(root, out, key, skip_dirs=(), max_bytes=2_000_000):
    wr = Writer(out, key)
    skipped = 0
    for d, dirs, files in sorted(os.walk(root)):
        dirs[:] = sorted(x for x in dirs if x not in skip_dirs)
        rel = os.path.relpath(d, root)
        cats = [] if rel == "." else re.split(r"[/_ -]+", rel)
        for f in sorted(files):
            if not f.endswith(".svg"):
                continue
            raw = open(os.path.join(d, f), "rb").read()
            if len(raw) > max_bytes:
                skipped += 1
                continue
            svg = slim(raw.decode("utf-8", "replace"))
            if "<svg" not in svg:
                continue
            name = re.sub(r"[_-]+", " ", f[:-4])
            wr.add(name, cats + name.split(), svg)
    wr.close()
    return wr.count, skipped


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--npm", required=True)
    ap.add_argument("--vendor", required=True)
    ap.add_argument("--tango", required=True, help="tango-icon-theme-0.8.90/scalable")
    ap.add_argument("--openclipart", required=True, help="openclipart 0.18 svg directory")
    ap.add_argument("--people", help="DiceBear output (people.json) for C21-C23")
    ap.add_argument("--out", required=True)
    a = ap.parse_args()
    os.makedirs(a.out, exist_ok=True)
    sets = []
    for sid, prefix, group, tint in ICONIFY:
        sets.append(build_iconify(a.npm, a.out, sid, prefix, group, tint))
        print(sid, prefix, sets[-1]["count"], file=sys.stderr)
    if a.people:
        people = json.load(open(a.people, encoding="utf-8"))
        for p in people:
            wr = Writer(a.out, p["key"])
            for item in p["items"]:
                wr.add(item["name"], item["keywords"], item["svg"])
            wr.close()
            meta = {k: v for k, v in p.items() if k != "items"}
            meta["count"] = wr.count
            sets.append(meta)
            print(p["id"], p["key"], wr.count, file=sys.stderr)
    sets.append(build_humaaans(a.vendor, a.out))
    sets.append(build_opendoodles(a.vendor, a.out))
    n, _ = build_tree(a.tango, a.out, "tango")
    sets.append({
        "id": "C36", "key": "tango", "name": "Tango Desktop Project", "group": "classic", "tint": "",
        "author": "The Tango Desktop Project", "author_url": "http://tango.freedesktop.org",
        "licence": "Public domain (0.8.90)", "licence_spdx": "", "licence_url": "",
        "credit": False, "count": n,
    })
    # Company logos are left out: they are trademarks whatever the file's licence.
    n, skipped = build_tree(a.openclipart, a.out, "openclipart", skip_dirs=("logos",))
    sets.append({
        "id": "C26", "key": "openclipart", "name": "Openclipart", "group": "classic", "tint": "",
        "author": "Openclipart contributors", "author_url": "https://openclipart.org",
        "licence": "Public domain", "licence_spdx": "CC0-1.0", "licence_url": "https://openclipart.org/share",
        "credit": False, "count": n,
    })
    print("openclipart skipped (over 2 MB):", skipped, file=sys.stderr)
    for s in sets:
        print(s["id"], s["key"], s["count"], file=sys.stderr)
    json.dump(sets, open(os.path.join(a.out, "sets.json"), "w", encoding="utf-8"), indent=1, ensure_ascii=False)


if __name__ == "__main__":
    main()
