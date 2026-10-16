#!/usr/bin/env python3
from pathlib import Path
import re
import sys

if len(sys.argv) != 2 or not re.fullmatch(r"v[0-9]+\.[0-9]+\.[0-9]+", sys.argv[1]):
    sys.exit("usage: stamp-version.py vMAJOR.MINOR.PATCH")
path = Path("Cargo.toml")
text = path.read_text()
text, count = re.subn(r'(\[workspace.package\]\nversion = ")[^"]+("\n)', rf'\g<1>{sys.argv[1][1:]}\2', text)
if count != 1:
    sys.exit("cannot determine workspace version")
path.write_text(text)
