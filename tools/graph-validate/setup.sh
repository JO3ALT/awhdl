#!/bin/sh
# Install the development-only validators outside the repository (default
# ~/.cache/awhdl/graph-validate, override with AWHDL_GRAPH_TOOLS): Node packages
# (Mermaid, headless Chrome through Puppeteer, Graphviz WASM), Python packages
# (pydot, pm4py) in a virtual environment, and the pinned, checksum-verified
# PlantUML jar. Needs Node 18+, Python 3.10+, Java 11+ and curl.
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
TOOLS=${AWHDL_GRAPH_TOOLS:-$HOME/.cache/awhdl/graph-validate}
mkdir -p "$TOOLS"
cp "$HERE/package.json" "$HERE/requirements.txt" "$HERE"/*.mjs "$HERE"/*.py "$TOOLS/"
cd "$TOOLS"
npm install --no-audit --no-fund
python3 -m venv .venv
.venv/bin/pip install --quiet -r requirements.txt
sh "$HERE/fetch-plantuml.sh" "$TOOLS"
echo "installed in $TOOLS"
