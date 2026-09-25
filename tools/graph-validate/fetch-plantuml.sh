#!/bin/sh
# Download the pinned PlantUML jar into the given directory and verify it.
set -eu
DIR=${1:-${AWHDL_GRAPH_TOOLS:-$HOME/.cache/awhdl/graph-validate}}
VERSION=1.2025.4
SHA256=26518e14a3a04100cd76c0d96cab2d1171f36152215edd9790a28d20268200c1
mkdir -p "$DIR"
cd "$DIR"
if [ ! -f plantuml.jar ]; then
    curl -sSL -o plantuml.jar.part \
        "https://github.com/plantuml/plantuml/releases/download/v$VERSION/plantuml-$VERSION.jar"
    mv plantuml.jar.part plantuml.jar
fi
echo "$SHA256  plantuml.jar" | sha256sum -c -
