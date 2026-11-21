#!/usr/bin/env python3
import importlib.util
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / "comment-ratchet.py"
spec = importlib.util.spec_from_file_location("comment_ratchet", SCRIPT)
ratchet = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ratchet)


def counted(path, src):
    return [c.text.strip() for c in ratchet.comments(path, src) if not c.allowed]


def allowed(path, src):
    return [(c.text.strip(), c.allowed) for c in ratchet.comments(path, src) if c.allowed]


class LexerTests(unittest.TestCase):
    def test_rust_skips_strings_chars_and_lifetimes_and_nests_blocks(self):
        src = r'''let url = "https://a//b"; // one
let raw = r#"// no "quoted" "#;
let bytes = b"//";
let slash = '/'; let quote = '"'; let tick = '\'';
fn f<'a>(x: &'a str) -> &'a str { x }
/* two /* nested */ still two */
let after = 1; /* three */
'''
        self.assertEqual(counted("crates/x/src/lib.rs", src),
                         ["// one", "/* two /* nested */ still two */", "/* three */"])

    def test_swift_skips_interpolation_raw_and_multiline_strings(self):
        src = r'''let a = "\(flag ? "x // no" : "y") // no"
let b = #"raw // no \(not) "#
let c = """
    // no
    """
// yes
#if DEBUG
#endif
'''
        self.assertEqual(counted("Collie/Sources/A.swift", src), ["// yes"])

    def test_javascript_skips_templates_and_regular_expressions(self):
        src = r'''const t = `${'//'} // no ${`${"//"}`}`;
const re = /[//]+\/*x/g;
const u = 'http://x'; // yes
const q = a / b / c; /* also */
'''
        self.assertEqual(counted("relay/src/a.ts", src), ["// yes", "/* also */"])

    def test_python_counts_comments_and_docstrings(self):
        src = '#!/usr/bin/env python3\n"""Module."""\nx = "# no"  # yes\n\n\ndef f():\n    """Doc."""\n    return 1\n'
        self.assertEqual(counted("scripts/a.py", src), ['"""Module."""', "# yes", '"""Doc."""'])
        self.assertEqual(allowed("scripts/a.py", src), [("#!/usr/bin/env python3", "shebang")])

    def test_shell_skips_quotes_expansions_and_heredocs(self):
        src = r'''#!/usr/bin/env bash
echo "# no $(printf '%s' "# no")" '# no' ${#a} $# x#y
cat <<'EOF'
# no
EOF
# yes
echo hi # trailing
'''
        self.assertEqual(counted("scripts/a.sh", src), ["# yes", "# trailing"])

    def test_yaml_reads_run_blocks_as_shell_and_allows_action_versions(self):
        sha = "3d3c42e5aac5ba805825da76410c181273ba90b1"
        src = f'''# top
name: "a # no"
jobs:
  build:
    steps:
      - uses: actions/checkout@{sha} # v7.0.1
      - uses: actions/cache@v4 # v4
      - name: restore Xcode's cache
        with:
          path: |
            # no
      - run: |
          echo "# no"
          # yes
'''
        self.assertEqual(counted(".github/workflows/a.yml", src), ["# top", "# v4", "# yes"])
        self.assertEqual(allowed(".github/workflows/a.yml", src), [("# v7.0.1", "action-version")])

    def test_justfile_allows_recipe_docs_and_recipe_shebangs(self):
        src = '''x := "a"

# about the next line only
# Builds it.
[macos]
build target="all" profile="release": deps
    #!/usr/bin/env bash
    # inside
    echo {{ target }}
'''
        self.assertEqual(counted("justfile", src), ["# about the next line only", "# inside"])
        self.assertEqual(allowed("justfile", src), [("# Builds it.", "data"), ("#!/usr/bin/env bash", "shebang")])


class ClassTests(unittest.TestCase):
    def test_doc_comments_that_become_data(self):
        src = r'''/// counted: a function
pub fn f() {}

/// A frame.
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Frame {
    /// Its id.
    pub id: u32,
    // counted: not a doc comment
}

#[uniffi::export]
impl Core {
    /// Exported.
    pub fn go(&self) {}
}

#[derive(Subcommand)]
enum Command {
    /// Run the daemon.
    Run,
}
'''
        self.assertEqual(counted("crates/x/src/lib.rs", src),
                         ["/// counted: a function", "// counted: not a doc comment"])
        self.assertEqual([a for _, a in allowed("crates/x/src/lib.rs", src)], ["data"] * 4)

    def test_directives(self):
        cases = [
            ("Collie/Package.swift", "// swift-tools-version:6.2\n"),
            ("Collie/A.swift", "// MARK: - Layout\n"),
            ("Collie/A.swift", "// swiftlint:disable:next force_cast\n"),
            ("scripts/a.sh", "# shellcheck disable=SC2086\n"),
            ("crates/x/a.go", "//go:build linux\n"),
        ]
        for path, src in cases:
            self.assertEqual([a for _, a in allowed(path, src)], ["directive"], src)
        self.assertEqual(counted("Collie/A.swift", "// TODO: later\n"), ["// TODO: later"])

    def test_adr_pointers_name_an_existing_record_and_nothing_else(self):
        record = next((ratchet.ROOT / "docs/adr").glob("0001-*.md")).name[:4]
        self.assertEqual(allowed("crates/x/src/lib.rs", f"// ADR-{record}\nfn f() {{}}\n"), [(f"// ADR-{record}", "adr")])
        self.assertEqual(allowed("scripts/a.sh", f"# ADR-{record}\n"), [(f"# ADR-{record}", "adr")])
        for src in ["// ADR-9999\n", f"// ADR-{record}: why\n", f"f(); // ADR-{record}\n"]:
            self.assertEqual(len(counted("crates/x/src/lib.rs", src)), 1, src)

    def test_pins_file_keeps_its_provenance(self):
        src = "# The tag this commit is.\nCOMMIT=abc\n"
        self.assertEqual(allowed("scripts/ghostty/pins.env", src), [("# The tag this commit is.", "pins")])
        self.assertEqual(counted("scripts/other.env", src), ["# The tag this commit is."])


class RatchetTests(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp())
        (self.root / "scripts").mkdir()
        shutil.copy(SCRIPT, self.root / "scripts/comment-ratchet.py")
        subprocess.run(["git", "init", "-q", str(self.root)], check=True)
        self.write("crates/a/src/lib.rs", "// one\n// two\nfn f() {}\n")
        self.write("crates/b/src/lib.rs", "// one\nfn g() {}\n")
        self.write("crates/a/tests/fixtures/x.rs", "// fixture\n")
        (self.root / "scripts/comment-baseline.txt").write_text("2 crates/a/src/lib.rs\n1 crates/b/src/lib.rs\n")

    def tearDown(self):
        shutil.rmtree(self.root)

    def write(self, path, text):
        file = self.root / path
        file.parent.mkdir(parents=True, exist_ok=True)
        file.write_text(text)
        subprocess.run(["git", "-C", str(self.root), "add", path], check=True)

    def run_ratchet(self, *args):
        return subprocess.run([sys.executable, str(self.root / "scripts/comment-ratchet.py"), *args],
                              capture_output=True, text=True)

    def baseline(self):
        return (self.root / "scripts/comment-baseline.txt").read_text()

    def test_matching_counts_pass(self):
        self.assertEqual(self.run_ratchet().returncode, 0)

    def test_a_raised_count_fails_and_update_refuses_it(self):
        self.write("crates/a/src/lib.rs", "// one\n// two\n// three\nfn f() {}\n")
        result = self.run_ratchet()
        self.assertEqual(result.returncode, 1)
        self.assertIn("crates/a/src/lib.rs: 3 comment lines, baseline 2", result.stderr)
        before = self.baseline()
        self.assertEqual(self.run_ratchet("--update").returncode, 1)
        self.assertEqual(self.baseline(), before)

    def test_a_new_file_must_have_none(self):
        self.write("crates/c/src/lib.rs", "fn h() {} // new\n")
        result = self.run_ratchet()
        self.assertEqual(result.returncode, 1)
        self.assertIn("crates/c/src/lib.rs: 1 comment lines, baseline 0", result.stderr)

    def test_a_lower_count_passes_and_update_lowers_the_baseline(self):
        self.write("crates/a/src/lib.rs", "// one\nfn f() {}\n")
        self.write("crates/b/src/lib.rs", "fn g() {}\n")
        result = self.run_ratchet()
        self.assertEqual(result.returncode, 0)
        self.assertIn("--update", result.stdout)
        self.assertEqual(self.run_ratchet("--update").returncode, 0)
        self.assertEqual(self.baseline(), "1 crates/a/src/lib.rs\n")
        self.assertEqual(self.run_ratchet().stdout, "")

    def test_untracked_and_out_of_scope_files_are_not_counted(self):
        (self.root / "crates/a/src/new.rs").write_text("// untracked\n")
        self.write("docs/notes.toml", "# config\n")
        self.assertEqual(self.run_ratchet().returncode, 0)


if __name__ == "__main__":
    unittest.main()
