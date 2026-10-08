"""Order 040: the third-party licences file the app shows in Settings > About > Licences (app/assets/licences.txt).

What it lists (everything Boyler Utilities ships or installs that is not its own code):
  - every crate compiled into the release exe: `cargo tree -p bu-app -e normal --target x86_64-pc-windows-msvc` (the
    exact command the release build uses, `cargo build --release -p bu-app`; build-only and dev-only crates are not in the
    exe and are left out, proc-macro crates are kept because the code they generate is), their licence files read from the
    cargo registry (~/.cargo/registry/src/*/<crate>-<version>/);
  - the Rust standard library (in every Rust exe);
  - Skia (the prebuilt skia-binaries rust-skia downloads: features jpegd, jpege, pdf, textlayout) and the third-party
    libraries built into it on Windows (zlib, libpng, libjpeg-turbo, HarfBuzz, ICU, Wuffs, Expat - FreeType is not used on
    Windows), with the licence files of the exact commits Skia's DEPS pins (m153);
  - the setup: Inno Setup (the setup exe is made with it) and Everything (the setup can download and install it);
  - the add-on: Raw Accel (the Mouse page downloads and installs its official release).
The texts that are not in the cargo registry are kept in tools/licences/extra/ (where each came from: NOTES below).

A licence offered as a choice ("MIT OR Apache-2.0") is used as MIT (preference: MIT, Zlib, Apache-2.0, ...). Every distinct
licence text is written ONCE; each part names the text(s) it uses and keeps its own copyright lines (cargo-about's way).

Format of the output (UTF-8, LF):
  # comment
  P<TAB>group<TAB>name<TAB>version<TAB>licence as declared<TAB>text id[,text id...]
  C<TAB>text id<TAB>a copyright line of the part above (shown with that text)
  T<TAB>text id<TAB>title
  |<one line of the text above>

Usage (from the repo root, Git Bash):  CARGO_TARGET_DIR=... py -3 -I tools/licences/gen.py
Check only (exit 1 when the committed file is stale):  py -3 -I tools/licences/gen.py --check
"""

import json
import os
import re
import subprocess
import sys
import textwrap

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), '..', '..'))
EXTRA = os.path.join(os.path.dirname(os.path.abspath(__file__)), 'extra')
OUT = os.path.join(ROOT, 'app', 'assets', 'licences.txt')
TARGET = 'x86_64-pc-windows-msvc'

# NOTES - where the texts in extra/ come from:
#   skia.txt                  LICENSE_SKIA of the prebuilt skia-binaries (skia-bindings 0.153.3 OUT_DIR)
#   rust-skia.txt             github.com/rust-skia/rust-skia tag 0.153.3 LICENSE (skia-safe / skia-bindings ship none)
#   taffy.txt                 github.com/DioxusLabs/taffy tag v0.14.0 LICENSE (the crate ships none)
#   zlib.txt ... expat.txt    the LICENSE / COPYING files of the commits pinned in github.com/rust-skia/skia tag
#                             m153-0.101.2 DEPS (chromium / skia googlesource mirrors)
#   everything.txt            www.voidtools.com/License.txt (Everything 1.4, the MSI the setup installs)
#   innosetup.txt             license.txt of Inno Setup 6.7.0 (the one tools/installer/build.sh uses)
#   rawaccel.txt              LICENSE inside the official RawAccel_v1.7.1.zip (the one crates/addons pins)
#   rust-unicode.txt          the Rust toolchain's share/doc/rust/licenses/Unicode-3.0.txt (library/core/src/unicode)

# The parts that are not crates: (group, name, version, declared licence, [(text file, licence id)], extra copyright lines)
SKIA = 'Skia and the libraries built into it'
SETUP = 'Setup'
ADDONS = 'Add-ons'
RUST = 'Rust standard library and crates'
EXTRA_PARTS = [
    (SKIA, 'Skia', 'm153', 'BSD-3-Clause', [('skia.txt', 'BSD-3-Clause')], []),
    (SKIA, 'zlib (Chromium)', '1.3.0.1', 'Zlib', [('zlib.txt', None)], []),
    (SKIA, 'libpng', '1.6.56', 'libpng-2.0', [('libpng.txt', None)], []),
    (SKIA, 'libjpeg-turbo', '3.1.0', 'IJG AND BSD-3-Clause AND Zlib', [('libjpeg-turbo.txt', None), ('libjpeg-turbo-ijg.txt', None, 'README.ijg · the IJG License')], []),
    (SKIA, 'HarfBuzz', '13.1.0', 'MIT-Modern-Variant', [('harfbuzz.txt', None)], []),
    (SKIA, 'ICU', '78.2', 'Unicode-3.0', [('icu.txt', None)], []),
    (SKIA, 'Wuffs', '0.3.3', 'Apache-2.0', [('wuffs.txt', None)], []),
    (SKIA, 'Expat', '2.7.4', 'MIT', [('expat.txt', 'MIT')], []),
    (SETUP, 'Inno Setup', '6.7.0', 'Inno Setup License', [('innosetup.txt', None)], []),
    (SETUP, 'Everything (voidtools)', '1.4.1.1032', 'MIT AND BSD-3-Clause (PCRE)', [('everything.txt', None)], []),
    (ADDONS, 'Raw Accel', '1.7.1', 'MIT', [('rawaccel.txt', 'MIT')], []),
]
# crates that ship no licence file: the file of their repository at that version (extra/)
CRATE_FILES = {
    'skia-safe': 'rust-skia.txt',
    'skia-bindings': 'rust-skia.txt',
    'taffy': 'taffy.txt',
}
PREFER = ['MIT', 'Zlib', 'Apache-2.0', 'BSD-3-Clause', 'Unicode-3.0', 'Unlicense', '0BSD']
TITLES = {
    'MIT': 'MIT License',
    'Zlib': 'zlib License',
    'Apache-2.0': 'Apache License 2.0',
    'BSD-3-Clause': 'BSD 3-Clause License',
    'Unicode-3.0': 'Unicode License v3',
    'Unlicense': 'The Unlicense',
    '0BSD': 'BSD Zero Clause License',
}
# the file names that hold each licence inside a crate (lower case, without extension)
FILE_NAMES = {
    'MIT': ['license-mit', 'license_mit', 'mit-license'],
    'Apache-2.0': ['license-apache', 'license-apache-2.0', 'license_apache'],
    'Zlib': ['license-zlib'],
    'Unicode-3.0': ['license-unicode'],
    'Unlicense': ['unlicense'],
    '0BSD': ['license-0bsd'],
}

COPYRIGHT = re.compile(r'^\s*[*#]?\s*((portions\s+)?copyright\s*(\(c\)|©|\d|:|\[)|\(c\)\s|©)', re.I)
TITLE = re.compile(r'^\s*(the\s+)?mit\s+license(\s*\(mit\))?\s*$', re.I)


def run(args):
    r = subprocess.run(args, cwd=ROOT, capture_output=True, text=True, encoding='utf-8')
    if r.returncode != 0:
        sys.exit(f'{" ".join(args)} failed:\n{r.stderr}')
    return r.stdout


def read(path):
    with open(path, encoding='utf-8-sig') as f:
        t = f.read()
    return t.replace('\r\n', '\n').replace('\r', '\n')


def tidy(text):
    """Tabs to spaces, no trailing blanks, common indentation and blank edges removed, at most one blank line in a row."""
    lines = [l.expandtabs(4).rstrip() for l in text.split('\n')]
    t = textwrap.dedent('\n'.join(lines)).strip('\n')
    return re.sub(r'\n{3,}', '\n\n', t)


def split_copyright(text):
    """(copyright lines, the text without them and without an 'MIT License' title)."""
    cr, body = [], []
    for l in text.split('\n'):
        if COPYRIGHT.match(l):
            cr.append(l.strip().lstrip('*#').strip())
        elif TITLE.match(l):
            continue
        else:
            body.append(l)
    return cr, tidy('\n'.join(body))


def norm(text):
    t = text.lower().replace('“', '"').replace('”', '"').replace('’', "'")
    return re.sub(r'\s+', ' ', t).strip()


def choose(expr):
    """The licences used from an SPDX expression: every AND term, the preferred one of each OR choice."""
    expr = expr.replace('/', ' OR ')
    out = []
    depth, cur, terms = 0, '', []
    for tok in re.findall(r'\(|\)|[^\s()]+', expr):
        if tok == '(':
            depth += 1
        elif tok == ')':
            depth -= 1
        if tok == 'AND' and depth == 0:
            terms.append(cur)
            cur = ''
        else:
            cur += ' ' + tok
    terms.append(cur)
    for t in terms:
        opts = [o.strip('() ') for o in re.split(r'\bOR\b', t)]
        opts = [o for o in opts if o]
        best = sorted(opts, key=lambda o: PREFER.index(o) if o in PREFER else 99)[0]
        out.append(best)
    return out


def crate_file(d, lic, name):
    files = os.listdir(d)
    base = {f: os.path.splitext(f)[0].lower() for f in files}
    for f in files:
        if base[f] in FILE_NAMES.get(lic, []):
            return os.path.join(d, f)
    # a crate with one licence: its plain LICENSE / COPYING file
    for f in files:
        if base[f] in ('license', 'copying', 'licence'):
            t = read(os.path.join(d, f))
            if lic == 'MIT' and 'Permission is hereby granted' in t:
                return os.path.join(d, f)
            if lic == 'Zlib' and "provided 'as-is'" in t:
                return os.path.join(d, f)
    if name in CRATE_FILES:
        return os.path.join(EXTRA, CRATE_FILES[name])
    sys.exit(f'no {lic} licence file for {name} in {d}')


class Texts:
    """Every distinct licence text once; a part gets the id of the text its file matches."""

    def __init__(self):
        self.texts = []  # (id, title, body)
        self.by_norm = {}
        # MIT's standard wording first (Raw Accel's file is the SPDX text word for word)
        _, body = split_copyright(tidy(read(os.path.join(EXTRA, 'rawaccel.txt'))))
        self.add('MIT', TITLES['MIT'], body)

    def add(self, tid, title, body):
        self.texts.append((tid, title, body))
        self.by_norm[norm(body)] = tid
        return tid

    def place(self, lic, raw, own_title, part=''):
        """(text id, copyright lines). A known licence (lic) has its copyright lines taken out and shares its text with
        every other part whose remaining text is the same; anything else is kept whole as the part's own text."""
        raw = tidy(raw)
        if lic is None:
            n = norm(raw)
            if n in self.by_norm:
                return self.by_norm[n], []
            return self.add(slug(f'{part} {own_title}'), own_title, raw), []
        cr, body = split_copyright(raw)
        n = norm(body)
        if n in self.by_norm:
            return self.by_norm[n], cr
        ids = [t[0] for t in self.texts]
        tid = lic if lic not in ids else next(f'{lic}~{k}' for k in range(2, 99) if f'{lic}~{k}' not in ids)
        title = TITLES.get(lic, lic) + ('' if tid == lic else f' ({part})')
        return self.add(tid, title, body), cr


def slug(s):
    return re.sub(r'[^a-z0-9]+', '-', s.lower()).strip('-')


def crates():
    tree = run(['cargo', 'tree', '-p', 'bu-app', '-e', 'normal', '--target', TARGET, '--prefix', 'none', '--offline'])
    seen = []
    for line in tree.splitlines():
        m = re.match(r'^(\S+) v(\S+)(.*)$', line.strip())
        if not m:
            continue
        key = (m.group(1), m.group(2))
        if key not in seen:
            seen.append(key)
    meta = json.loads(run(['cargo', 'metadata', '--format-version', '1', '--offline', '--filter-platform', TARGET]))
    pk = {(p['name'], p['version']): p for p in meta['packages']}
    out = []
    for key in seen:
        p = pk[key]
        if p['source'] is None:
            # a path crate of this workspace (bu-*): our own code
            continue
        out.append(p)
    return sorted(out, key=lambda p: (p['name'], [int(x) if x.isdigit() else x for x in re.split(r'[.+-]', p['version'])]))


def build():
    texts = Texts()
    parts = []  # (group, name, version, declared, [ids], [copyright])

    def part(group, name, version, declared, files):
        ids, crs = [], []
        for path, lic, title in files:
            tid, cr = texts.place(lic, read(path), title, name)
            if tid not in ids:
                ids.append(tid)
            crs += [(tid, c) for c in cr if (tid, c) not in crs]
        parts.append((group, name, version, declared, ids, crs))

    for g, name, ver, declared, files, _ in EXTRA_PARTS:
        if g == SKIA:
            part(g, name, ver, declared, [(os.path.join(EXTRA, f[0]), f[1], f[2] if len(f) > 2 else 'Licence') for f in files])
    # the Rust standard library (its COPYRIGHT-library notice: Apache-2.0 OR MIT, core's Unicode tables Unicode-3.0)
    rustc = run(['rustc', '--version']).split()[1]
    tid_mit = 'MIT'
    tid_uni, cr_uni = texts.place('Unicode-3.0', read(os.path.join(EXTRA, 'rust-unicode.txt')), 'Rust standard library')
    parts.append((RUST, 'Rust standard library', rustc, 'MIT OR Apache-2.0, Unicode-3.0', [tid_mit, tid_uni],
                  [(tid_mit, 'Copyright: The Rust Project Developers (see https://thanks.rust-lang.org)')] + [(tid_uni, c) for c in cr_uni]))
    for p in crates():
        d = os.path.dirname(p['manifest_path'])
        files = [(crate_file(d, lic, p['name']), lic, p['name']) for lic in choose(p['license'])]
        part(RUST, p['name'], p['version'], p['license'], files)
    for g, name, ver, declared, files, _ in EXTRA_PARTS:
        if g != SKIA:
            part(g, name, ver, declared, [(os.path.join(EXTRA, f[0]), f[1], f[2] if len(f) > 2 else 'Licence') for f in files])

    used = {t for p in parts for t in p[4]}
    out = ['# Boyler Utilities - the third-party parts it ships and their licences (Settings > About > Licences).',
           '# Generated by tools/licences/gen.py - do not edit by hand.']
    for g, name, ver, declared, ids, crs in parts:
        out.append('\t'.join(['P', g, name, ver, declared, ','.join(ids)]))
        out += [f'C\t{t}\t{c}' for t, c in crs]
    for tid, title, body in texts.texts:
        if tid not in used:
            continue
        out.append(f'T\t{tid}\t{title}')
        out += ['|' + l for l in body.split('\n')]
    return '\n'.join(out) + '\n', parts, texts


def main():
    data, parts, texts = build()
    if '--check' in sys.argv:
        old = read(OUT) if os.path.exists(OUT) else ''
        if old != data:
            sys.exit('app/assets/licences.txt is stale: run tools/licences/gen.py')
        print('licences.txt up to date')
        return
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, 'w', encoding='utf-8', newline='\n') as f:
        f.write(data)
    n_crates = sum(1 for p in parts if p[0] == RUST) - 1
    print(f'{OUT}: {len(data.encode("utf-8"))} bytes, {len(parts)} parts ({n_crates} crates), '
          f'{sum(1 for t in texts.texts if any(t[0] in p[4] for p in parts))} licence texts')


if __name__ == '__main__':
    main()
