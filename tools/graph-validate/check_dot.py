"""Parse Graphviz DOT files with pydot."""
import sys

import pydot

failed = 0
for path in sys.argv[1:]:
    if not pydot.graph_from_dot_file(path):
        failed += 1
        print("FAIL", path)
print(f"dot: {len(sys.argv) - 1 - failed} ok, {failed} failed")
sys.exit(1 if failed else 0)
