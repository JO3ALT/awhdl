# Validators for aic graph (development only)

English | [日本語](README_ja.md)

> The Japanese version is the normative (canonical) document. This English version is a reference translation; if they differ, the Japanese version prevails.

Development-only tools that check `aic graph` output with real diagram tools. Every example is drawn in
every view and format, and each output must parse; `--render` also renders PNGs.

| Format | Check | Rendering (`--render`) |
|---|---|---|
| Mermaid | the Mermaid library's `mermaid.parse` | Mermaid in headless Chrome (Puppeteer) |
| DOT | pydot | Graphviz WASM (@hpcc-js/wasm-graphviz) |
| PlantUML | `plantuml -checkonly` | PlantUML |
| PNML | pm4py `read_pnml` (loaded as a P/T net) | — |
| JSON | `python3 -m json.tool` | — |

## Usage

```bash
sh tools/graph-validate/setup.sh             # once; needs Node 18+, Python 3.10+, Java 11+, curl
sh tools/graph-validate/validate.sh          # check
sh tools/graph-validate/validate.sh --render # check and render PNGs
AIC=target/debug/aic sh tools/graph-validate/validate.sh   # use a built aic
```

Tools and outputs live outside the repository by default (`~/.cache/awhdl/graph-validate`, override
with `AWHDL_GRAPH_TOOLS`), so no `node_modules` ends up in a synced folder such as OneDrive.

## Pinned versions and licenses

Versions are pinned in `package.json`, `requirements.txt` and `fetch-plantuml.sh` (verified by SHA-256).
They are not vendored; `setup.sh` installs them. All are used for development checks only and are not
dependencies of `aic` or the runtime.

| Tool | Version | License |
|---|---|---|
| mermaid | 12.0.0 | MIT |
| puppeteer (fetches headless Chrome) | 22.15.0 | Apache-2.0 (Chrome under its own terms) |
| @hpcc-js/wasm-graphviz | 1.29.1 | Apache-2.0 (Graphviz under EPL) |
| jsdom / dompurify | 22.1.0 / 3.4.16 | MIT / Apache-2.0 or MPL-2.0 |
| pydot | 4.0.1 | MIT |
| pm4py | 2.7.23.8 | AGPL-3.0 |
| PlantUML | 1.2025.4 | GPL-3.0 |

pm4py (AGPL) and PlantUML (GPL) are only run locally for checks and are never distributed.
