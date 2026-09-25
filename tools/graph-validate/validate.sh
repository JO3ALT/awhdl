#!/bin/sh
# Draw every example in every view and format with aic graph, then check that
# each output parses: Mermaid (mermaid.parse), DOT (pydot), PlantUML
# (plantuml -checkonly), PNML (pm4py) and JSON. With --render, also render
# PNGs (Mermaid in headless Chrome, DOT with Graphviz WASM, PlantUML).
# Run setup.sh once first. Usage: sh tools/graph-validate/validate.sh [--render]
# Outputs go to $TOOLS/out (outside the repository).
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
TOOLS=${AWHDL_GRAPH_TOOLS:-$HOME/.cache/awhdl/graph-validate}
AIC=${AIC:-"cargo run -q -p conductor-cli --"}
OUT="$TOOLS/out"
rm -rf "$OUT"; mkdir -p "$OUT"
cd "$ROOT"
for file in examples/*.awhdl examples/tutorial/0[1-5]*.awhdl tests/graph/*.awhdl; do
    name=$(basename "$file" .awhdl)
    for view in structure behavior petri security activity; do
        for options in "" "--compact" "--show-classification --show-capabilities --show-policies --show-internal"; do
            tag=$view$(echo "$options" | tr -d ' -' | cut -c1-12)
            $AIC graph "$file" --view $view --format mermaid $options --output "$OUT/$name.$tag.mmd"
            $AIC graph "$file" --view $view --format dot $options --output "$OUT/$name.$tag.dot"
            $AIC graph "$file" --view $view --format json $options --output "$OUT/$name.$tag.json"
            if [ $view = activity ]; then
                $AIC graph "$file" --view $view --format plantuml $options --output "$OUT/$name.$tag.puml"
            fi
            if [ $view = petri ]; then
                $AIC graph "$file" --view $view --format pnml $options --output "$OUT/$name.$tag.pnml"
            fi
        done
    done
done
node "$TOOLS/check_mermaid.mjs" "$OUT"/*.mmd
"$TOOLS/.venv/bin/python" "$TOOLS/check_dot.py" "$OUT"/*.dot
for json in "$OUT"/*.json; do python3 -m json.tool "$json" > /dev/null; done
echo "json: $(ls "$OUT"/*.json | wc -l) ok"
java -jar "$TOOLS/plantuml.jar" -checkonly "$OUT"/*.puml
echo "plantuml: $(ls "$OUT"/*.puml | wc -l) ok"
"$TOOLS/.venv/bin/python" "$TOOLS/check_pnml.py" "$OUT"/*.pnml
if [ "${1:-}" = "--render" ]; then
    node "$TOOLS/render.mjs" "$OUT"/*.mmd "$OUT"/*.dot
    java -jar "$TOOLS/plantuml.jar" -tpng "$OUT"/*.puml
fi
