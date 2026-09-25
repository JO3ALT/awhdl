#!/bin/sh
# Rebuild the tutorial figures from the examples with aic graph and Graphviz.
# Run from the awhdl root:  sh docs/tutorial/figures/build.sh
# AIC may name a built aic binary; by default it is run through cargo.
set -eu
AIC=${AIC:-"cargo run -q -p conductor-cli --"}
OUT=docs/tutorial/figures

figure() {
    name=$1; shift
    $AIC graph "$@" --format dot | dot -Tpdf -o "$OUT/$name.pdf"
}

figure structure_02 examples/tutorial/02_prolog_verdict.awhdl --view structure --show-capabilities
figure behavior_05  examples/tutorial/05_parallel_barrier.awhdl --view behavior
figure activity_05  examples/tutorial/05_parallel_barrier.awhdl --view activity
figure petri_04     examples/tutorial/04_feedback_loop.awhdl --view petri
figure security     examples/secure_cooperation.awhdl --view security
figure petri_coop   examples/secure_cooperation.awhdl --view petri
