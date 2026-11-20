#!/usr/bin/env python3
import argparse
import ast
import bisect
import io
import re
import subprocess
import sys
import tokenize
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BASELINE = "scripts/comment-baseline.txt"

SCOPE = re.compile(
    r"^(crates/.+\.(rs|go)|(Collie|Mac)/.+\.swift|relay/.+\.ts|Linux/.+\.(js|cjs|mjs|qml)"
    r"|scripts/.+\.(py|sh|env)|docs/site/[^/]+\.sh|packaging/aur/(install\.sh|[^/]+/collie-tray-activate|[^/]+/collied-aur-adopt)"
    r"|justfile|\.github/workflows/[^/]+\.ya?ml)$"
)
EXCLUDED = re.compile(
    r"(^|/)fixtures/|^Linux/CollieTray/Printable\.js$"
    r"|^Collie/GhosttyTerminal/Sources/GhosttyTerminal/NerdFontConstraints\.swift$"
)
PINS = {"scripts/ghostty/pins.env"}

LANGUAGES = {".rs": "rust", ".go": "go", ".swift": "swift", ".ts": "js", ".js": "js", ".cjs": "js", ".mjs": "js",
             ".qml": "js", ".py": "python", ".sh": "shell", ".env": "shell", ".yml": "yaml", ".yaml": "yaml"}

DIRECTIVE = re.compile(r"^(//|#)\s*(swift-tools-version:|swiftlint:|MARK:|shellcheck |go:build )")
ADR = re.compile(r"^(//|#)\s*ADR-(\d{4})$")
ACTION_PIN = re.compile(r"^\s*(- )?uses:\s*\S+@[0-9a-f]{40}\s+$")
ACTION_VERSION = re.compile(r"^#\s*v?\d[\w.+-]*$")
DATA_DERIVE = re.compile(
    r"derive\([^)]*\b(JsonSchema|Parser|Subcommand|Args|ValueEnum|uniffi::(Record|Enum|Object|Error))\b|#\[uniffi::export\b"
)
JUST_RECIPE = re.compile(r"^@?[A-Za-z_][\w-]*(\s[^:]*)?:(?!=)")
JS_REGEX_AFTER = set("(,=:[!&|?{};+-*%<>~^")
JS_KEYWORDS = {"return", "typeof", "case", "do", "else", "in", "of", "new", "delete", "void", "throw", "yield", "await"}


@dataclass
class Comment:
    line: int
    end: int
    text: str
    trailing: bool
    allowed: str = ""


def language(path):
    name = Path(path).name
    if name == "justfile":
        return "just"
    suffix = Path(path).suffix
    return LANGUAGES.get(suffix, "shell")


class CLexer:
    def __init__(self, src, lang):
        self.s = src
        self.lang = lang
        self.spans = []

    def run(self):
        self.code(0, None)
        return self.spans

    def code(self, i, close):
        s, n, depth, prev = self.s, len(self.s), 0, ""
        while i < n:
            c = s[i]
            if s.startswith("//", i):
                j = s.find("\n", i)
                j = n if j < 0 else j
                self.spans.append((i, j))
                i = j
                continue
            if s.startswith("/*", i):
                j = self.block(i)
                self.spans.append((i, j))
                i = j
                continue
            if c in "([{":
                depth += 1
            elif c in ")]}":
                if depth == 0 and c == close:
                    return i + 1
                depth -= 1
            j = self.literal(i, prev)
            if j is not None:
                i, prev = j, '"'
                continue
            if not c.isspace():
                prev = c if not (c.isalnum() or c in "_$") else self.word(i)
            i += 1
        return i

    def word(self, i):
        j = i
        while j + 1 < len(self.s) and (self.s[j + 1].isalnum() or self.s[j + 1] in "_$"):
            j += 1
        k = i
        while k > 0 and (self.s[k - 1].isalnum() or self.s[k - 1] in "_$"):
            k -= 1
        return self.s[k:j + 1]

    def block(self, i):
        s, depth, i = self.s, 1, i + 2
        nests = self.lang in ("rust", "swift")
        while i < len(s) and depth:
            if nests and s.startswith("/*", i):
                depth, i = depth + 1, i + 2
            elif s.startswith("*/", i):
                depth, i = depth - 1, i + 2
            else:
                i += 1
        return i

    def ident_before(self, i):
        return i > 0 and (self.s[i - 1].isalnum() or self.s[i - 1] == "_")

    def literal(self, i, prev):
        s, c, lang = self.s, self.s[i], self.lang
        if lang == "rust":
            if c in "rb" and not self.ident_before(i):
                m = re.compile(r"b?r(#*)\"|b\"|b'").match(s, i)
                if m and m.group(0).startswith(("r", "br")):
                    end = s.find('"' + m.group(1), m.end())
                    return len(s) if end < 0 else end + 1 + len(m.group(1))
                if m and m.group(0) == 'b"':
                    return self.quoted(i + 1, '"', "\\")
                if m:
                    return self.char(i + 1)
            if c == '"':
                return self.quoted(i, '"', "\\")
            if c == "'":
                return self.char(i)
            return None
        if lang == "swift":
            m = re.compile(r'(#*)("""|")').match(s, i)
            if m and (m.group(1) or c == '"'):
                return self.swift_string(m.end(), m.group(2), m.group(1))
            return None
        if lang == "go":
            if c == "\x60":
                end = s.find("\x60", i + 1)
                return len(s) if end < 0 else end + 1
            if c in "\"'":
                return self.quoted(i, c, "\\")
            return None
        if c in "\"'":
            return self.quoted(i, c, "\\")
        if c == "\x60":
            return self.template(i + 1)
        if c == "/" and not s.startswith("//", i) and not s.startswith("/*", i):
            if prev == "" or prev in JS_REGEX_AFTER or prev in JS_KEYWORDS:
                return self.regex(i + 1)
        return None

    def quoted(self, i, quote, escape):
        s, i = self.s, i + 1
        while i < len(s):
            if s[i] == escape:
                i += 2
            elif s[i] == quote:
                return i + 1
            else:
                i += 1
        return i

    def char(self, i):
        s = self.s
        if i + 1 < len(s) and s[i + 1] == "\\":
            end = s.find("'", i + 3)
            return len(s) if end < 0 else end + 1
        if i + 2 < len(s) and s[i + 2] == "'":
            return i + 3
        return None

    def swift_string(self, i, delim, hashes):
        s, close, escape = self.s, delim + hashes, "\\" + hashes
        while i < len(s):
            if s.startswith(escape, i):
                j = i + len(escape)
                if j < len(s) and s[j] == "(":
                    i = self.code(j + 1, ")")
                else:
                    i = j + 1
            elif s.startswith(close, i):
                return i + len(close)
            else:
                i += 1
        return i

    def template(self, i):
        s = self.s
        while i < len(s):
            if s[i] == "\\":
                i += 2
            elif s[i] == "\x60":
                return i + 1
            elif s.startswith("${", i):
                i = self.code(i + 2, "}")
            else:
                i += 1
        return i

    def regex(self, i):
        s, in_class = self.s, False
        while i < len(s) and s[i] != "\n":
            if s[i] == "\\":
                i += 2
                continue
            if s[i] == "[":
                in_class = True
            elif s[i] == "]":
                in_class = False
            elif s[i] == "/" and not in_class:
                i += 1
                while i < len(s) and s[i].isalpha():
                    i += 1
                return i
            i += 1
        return i


def shell_spans(s, start=0, end=None):
    spans, end = [], len(s) if end is None else end

    def code(i, close):
        depth, pending = 0, []
        while i < end:
            c = s[i]
            if c == "\n" and pending:
                i = heredocs(i + 1, pending)
                pending = []
            elif c == "\\":
                i += 2
            elif c == "'":
                j = s.find("'", i + 1)
                i = end if j < 0 or j >= end else j + 1
            elif c == '"':
                i = dquote(i + 1)
            elif c == "\x60":
                j = s.find("\x60", i + 1)
                i = end if j < 0 or j >= end else j + 1
            elif c == "#" and (i == start or s[i - 1] in " \t\n;&|("):
                j = s.find("\n", i)
                j = end if j < 0 or j > end else j
                spans.append((i, j))
                i = j
            elif s.startswith("<<", i) and not s.startswith("<<<", i):
                m = re.compile(r"<<(-?)\s*(['\"]?)([A-Za-z_]\w*)\2").match(s, i)
                if m:
                    pending.append((m.group(3), bool(m.group(1))))
                    i = m.end()
                else:
                    i += 2
            elif c == "(":
                depth += 1
                i += 1
            elif c == ")":
                if depth == 0 and close:
                    return i + 1
                depth -= 1
                i += 1
            else:
                i += 1
        return i

    def dquote(i):
        while i < end:
            if s[i] == "\\":
                i += 2
            elif s[i] == '"':
                return i + 1
            elif s.startswith("$(", i):
                i = code(i + 2, True)
            else:
                i += 1
        return i

    def heredocs(i, pending):
        for word, tabs in pending:
            while i < end:
                j = s.find("\n", i)
                j = end if j < 0 else j
                line = s[i:j]
                i = j + 1
                if (line.lstrip("\t") if tabs else line) == word:
                    break
        return i

    code(start, False)
    return spans


def yaml_spans(s):
    spans, lines, pos = [], s.split("\n"), 0
    offsets = []
    for line in lines:
        offsets.append(pos)
        pos += len(line) + 1
    i = 0
    while i < len(lines):
        line = lines[i]
        col = yaml_comment(line)
        if col is not None:
            spans.append((offsets[i] + col, offsets[i] + len(line)))
        m = re.match(r"^(\s*)(- )?([\w.-]+):\s*[|>][-+0-9]*\s*(#.*)?$", line)
        if not m:
            i += 1
            continue
        indent = len(m.group(1)) + (2 if m.group(2) else 0)
        j = i + 1
        while j < len(lines) and (not lines[j].strip() or len(lines[j]) - len(lines[j].lstrip()) > indent):
            j += 1
        if m.group(3) == "run" and j > i + 1:
            spans += shell_spans(s, offsets[i + 1], offsets[j - 1] + len(lines[j - 1]))
        i = j
    return spans


def yaml_comment(line):
    i = 0
    while i < len(line):
        c = line[i]
        if c in "'\"" and (i == 0 or line[i - 1] in " [{,:"):
            i += 1
            while i < len(line):
                if c == '"' and line[i] == "\\":
                    i += 2
                elif line[i] == c and c == "'" and line.startswith("''", i):
                    i += 2
                elif line[i] == c:
                    break
                else:
                    i += 1
        elif c == "#" and (i == 0 or line[i - 1] in " \t"):
            return i
        i += 1
    return None


def python_comments(src):
    out = []
    lines = src.split("\n")
    for tok in tokenize.generate_tokens(io.StringIO(src).readline):
        if tok.type == tokenize.COMMENT:
            row, col = tok.start
            out.append(Comment(row, row, tok.string, bool(lines[row - 1][:col].strip())))
    for node in ast.walk(ast.parse(src)):
        if isinstance(node, (ast.Module, ast.ClassDef, ast.FunctionDef, ast.AsyncFunctionDef)) and node.body:
            first = node.body[0]
            if isinstance(first, ast.Expr) and isinstance(first.value, ast.Constant) and isinstance(first.value.value, str):
                out.append(Comment(first.lineno, first.end_lineno, ast.get_source_segment(src, first), False))
    return sorted(out, key=lambda c: c.line)


def comments(path, src):
    lang = language(path)
    if lang == "python":
        found = python_comments(src)
    else:
        if lang in ("rust", "swift", "js", "go"):
            spans = CLexer(src, lang).run()
        elif lang == "yaml":
            spans = yaml_spans(src)
        else:
            spans = shell_spans(src)
        starts = [0] + [i + 1 for i, c in enumerate(src) if c == "\n"]
        found = []
        for a, b in spans:
            line = bisect.bisect_right(starts, a)
            end = bisect.bisect_right(starts, max(a, b - 1))
            found.append(Comment(line, end, src[a:b], bool(src[starts[line - 1]:a].strip())))
    classify(path, lang, src, found)
    return found


def data_ranges(src):
    lines, ranges = src.split("\n"), []
    for i, line in enumerate(lines):
        if not DATA_DERIVE.search(line):
            continue
        top = i
        while top > 0 and lines[top - 1].strip().startswith(("///", "#[")):
            top -= 1
        j = i
        while j < len(lines) and lines[j].strip().startswith(("#[", "///")):
            j += 1
        depth, end, opened = 0, j, False
        for k in range(j, len(lines)):
            text = re.sub(r'"(\\.|[^"\\])*"|//.*', "", lines[k])
            depth += text.count("{") - text.count("}")
            opened = opened or "{" in text
            end = k
            if (opened and depth <= 0) or (not opened and text.rstrip().endswith(";")):
                break
        ranges.append((top + 1, end + 1))
    return ranges


def classify(path, lang, src, found):
    lines = src.split("\n")
    ranges = data_ranges(src) if lang == "rust" else []
    for c in found:
        text = c.text.strip()
        if text.startswith("#!") and not c.trailing and (c.line == 1 or lang == "just"):
            c.allowed = "shebang"
        elif DIRECTIVE.match(text):
            c.allowed = "directive"
        elif path in PINS:
            c.allowed = "pins"
        elif lang == "yaml" and c.trailing and ACTION_PIN.match(lines[c.line - 1][:-len(c.text)]) and ACTION_VERSION.match(text):
            c.allowed = "action-version"
        elif lang == "rust" and text.startswith("///") and not text.startswith("////") \
                and any(a <= c.line <= b for a, b in ranges):
            c.allowed = "data"
        elif lang == "just" and not c.trailing and lines[c.line - 1].startswith("#") and just_doc(lines, c.line):
            c.allowed = "data"
        elif not c.trailing and (m := ADR.match(text)) and list((ROOT / "docs/adr").glob(m.group(2) + "-*.md")):
            c.allowed = "adr"


def just_doc(lines, line):
    k = line
    while k < len(lines) and lines[k].startswith("["):
        k += 1
    return k < len(lines) and bool(JUST_RECIPE.match(lines[k]))


def tracked():
    out = subprocess.run(["git", "-C", str(ROOT), "ls-files", "-z"], check=True, capture_output=True, text=True).stdout
    return sorted(p for p in out.split("\0") if SCOPE.match(p) and not EXCLUDED.search(p) and (ROOT / p).is_file())


def counts(paths):
    result = {}
    for path in paths:
        found = [c for c in comments(path, (ROOT / path).read_text()) if not c.allowed]
        lines = {n for c in found for n in range(c.line, c.end + 1)}
        if lines:
            result[path] = len(lines)
    return result


def read_baseline():
    baseline = {}
    file = ROOT / BASELINE
    for row in file.read_text().splitlines() if file.exists() else []:
        count, path = row.split(" ", 1)
        baseline[path] = int(count)
    return baseline


def write_baseline(current):
    rows = [f"{n} {p}\n" for p, n in sorted(current.items())]
    (ROOT / BASELINE).write_text("".join(rows))


def check(update):
    current, baseline = counts(tracked()), read_baseline()
    raised = [(p, n, baseline.get(p, 0)) for p, n in current.items() if n > baseline.get(p, 0)]
    for path, n, was in raised:
        print(f"{path}: {n} comment lines, baseline {was}", file=sys.stderr)
    if raised:
        print("Delete the new comments: see .claude/skills/delete-comment/SKILL.md. The baseline never goes up.", file=sys.stderr)
        return 1
    if current == baseline:
        return 0
    if update:
        write_baseline(current)
        print(f"lowered {BASELINE}")
    else:
        print(f"Some files have fewer comment lines than {BASELINE}: lower it with python3 scripts/comment-ratchet.py --update")
    return 0


def listing(paths, everything):
    for path in paths:
        for c in comments(path, (ROOT / path).read_text()):
            if everything or not c.allowed:
                first = c.text.strip().split("\n")[0]
                print(f"{path}:{c.line}:{c.allowed or '-'}:{'trailing' if c.trailing else 'line'}:{first}")


def redundant_blanks(lines):
    count = 0
    for k, line in enumerate(lines):
        if line.strip():
            continue
        before = lines[k - 1].rstrip() if k else None
        after = lines[k + 1].strip() if k + 1 < len(lines) else ""
        if before is None or not before or before.endswith(("{", "(", "[")) or after.startswith(("}", ")", "]")):
            count += 1
    return count


def estimate(paths):
    rows = {}
    for path in paths:
        src = (ROOT / path).read_text()
        lines = src.split("\n")
        found = [c for c in comments(path, src) if not c.allowed]
        whole = {n for c in found if not c.trailing for n in range(c.line, c.end + 1)}
        trailing = [c for c in found if c.trailing]
        kept = [line for n, line in enumerate(lines, 1) if n not in whole]
        row = rows.setdefault(language(path), [0, 0, 0, 0, 0])
        row[0] += len(found)
        row[1] += len(whole)
        row[2] += len(trailing)
        row[3] += sum(len(c.text) for c in trailing)
        row[4] += max(0, redundant_blanks(kept) - redundant_blanks(lines))
    total = [0, 0, 0, 0, 0]
    print("language   comments  comment-only lines  trailing comments  trailing chars  blank lines")
    for lang, row in sorted(rows.items()):
        total = [a + b for a, b in zip(total, row)]
        print(f"{lang:9} {row[0]:9} {row[1]:19} {row[2]:18} {row[3]:15} {row[4]:12}")
    print(f"{'total':9} {total[0]:9} {total[1]:19} {total[2]:18} {total[3]:15} {total[4]:12}")
    print(f"lines shed: {total[1] + total[4]}")


def main(argv):
    parser = argparse.ArgumentParser(description="The comment ratchet: comments outside the allowed classes may only go down.")
    parser.add_argument("--update", action="store_true", help="lower the baseline to the current counts; never raises one")
    parser.add_argument("--list", action="store_true", help="print the comments the ratchet counts")
    parser.add_argument("--all", action="store_true", help="with --list, print the allowed comments too")
    parser.add_argument("--estimate", action="store_true", help="lines deleting every counted comment would shed")
    parser.add_argument("paths", nargs="*")
    args = parser.parse_args(argv)
    paths = tracked()
    if args.paths:
        paths = [p for p in paths if any(p == q or p.startswith(q.rstrip("/") + "/") for q in args.paths)]
    if args.list:
        listing(paths, args.all)
        return 0
    if args.estimate:
        estimate(paths)
        return 0
    return check(args.update)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
