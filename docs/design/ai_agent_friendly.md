# duckOSM for AI agents

**Status:** pieces 1–5 implemented 2026-10-01 (agreed with Kaveh: AGENTS.md holds the rules, CLAUDE.md imports it; a Claude plugin with the skill). Piece 6, the MCP server, waits for duckOSM on PyPI.

## Problem

More and more people will reach duckOSM through an AI agent (Claude Code, Cursor, Codex, ChatGPT…):
"build me a routable network of Tartu", "which roads changed", "export this to SUMO". Today an agent
has to guess: it reads the README, maybe crawls the docs site page by page, and doesn't know the
traps (the `edge_id` is a BIGINT hash, each mode is its own schema, private roads live in
`private_edges`, the empty `main` schema). Two kinds of agents matter:

- **Users' agents**: they install duckOSM and use it (build, query, route, export).
- **Coding agents**: they change duckOSM's own code. `CLAUDE.md` serves only Claude Code.

roadstyle already solved the first kind the same way this note proposes: an agent skill, an MCP
server and a Claude plugin.

## What agents need, and the piece that gives it

| Need | Piece | Size |
|---|---|---|
| know when and how to use duckOSM, without reading the whole site | **1. an agent skill** `skills/duckosm/SKILL.md` | small |
| read the docs as clean text, in one fetch | **2. `llms.txt`** on the docs site | tiny |
| see what a database holds before querying it | **3. `duckosm info DB`** (text or `--json`) | small |
| read command results without parsing tables | **4. `--json`** on the commands that print results | small |
| contribute to the code under the project's rules, in any coding tool | **5. `AGENTS.md`** | tiny |
| do it all without writing code or shell | **6. an MCP server** `duckosm-mcp` | medium, later |

### 1. Agent skill

`skills/duckosm/SKILL.md`, in the same shape as roadstyle's: a front-matter `description` that says
when to use it, then short sections:

- install, and the one command to a first network (`duckosm build -c` / `--pbf` / `--place`);
- the database contract: one schema per mode; `edges` (key columns), `nodes`, `edge_graph`,
  `private_edges`; `edge_id` is a stable BIGINT hash, so keep it BIGINT when joining; `(mode,
  edge_id)` across modes; always name the schema;
- recipes that work as written: the size of a network, the edge nearest a point, a route, one OSM
  way (`duckosm way`), an export;
- the traps (listed above) and where the full reference is (links to the site).

Shipped with the repo and listed in a `.claude-plugin/` manifest, like roadstyle, so Claude Code
users can install it as a plugin; any agent can also just read the file.

### 2. `llms.txt`

The [`mkdocs-llmstxt`](https://pawamoy.github.io/mkdocs-llmstxt) plugin (maintained; author of
mkdocstrings) writes `/llms.txt` (a short index of the site, the [llms.txt](https://llmstxt.org)
convention) and `/llms-full.txt` (every page as Markdown, in one file) at build time. One plugin in
`mkdocs.yml`, one pin in the docs install line and `docs.yml`. Design notes stay out (they are not
published anyway).

### 3. `duckosm info DB`

The first thing an agent (or a person) needs with an unknown database: what's in it.

```text
$ duckosm info monaco.duckdb
monaco.duckdb  (built 2026-10-01, duckOSM 0.1.0, time zone Europe/Monaco)
mode      edges   nodes  private  km     edge_graph  restrictions
driving   2,765   1,719  196      92.6   4,953       38
walking   10,952  4,116  154      244.1  ...
cycling   10,274  4,151  133      245.5  ...
also: raw (OSM data), features (10 layers), mm (across modes), boundary
```

`--json` gives the same as one JSON object. Read-only; it only counts rows and reads
`main.visualization_metadata`. That table holds the time zone but not the build date or the duckOSM
version, so a build adds two columns to it, `built_at` and `duckosm_version` (an older database
just shows them as unknown).

### 4. `--json`

On the commands that print a result for the reader to use: `info`, `way` (the edges table),
`route-lanes` (lanes, cost, maneuvers) and `gis-debug` (the check). The text
output stays the default. Commands that write files keep printing the file path; with `--json` they
print `{"out": "...", ...counts}`. Errors go to stderr with a non-zero exit code, as now.

### 5. `AGENTS.md`

[`AGENTS.md`](https://agents.md) is the file Codex, Cursor, Copilot, Gemini and others read for a
repo's rules. It gets what `CLAUDE.md` has (commands, architecture, invariants, conventions), and
`CLAUDE.md` shrinks to one line pointing to it plus anything Claude-specific, so the two never
disagree.

### 6. MCP server (later)

`duckosm-mcp`, like `roadstyle-mcp`: tools `build_area(place | bbox | pbf, modes)`, `info(db)`,
`query(db, sql)` (read-only connection), `way(db, osm_id)`, `nearest_edge(db, lat, lon, mode)`,
`route(db, from, to, mode)`, `export(db, format)`, `draw(db, mode)` (a roadstyle map + PNG).
Installed with `uvx duckosm-mcp`, which needs duckOSM on PyPI, still on hold for the DuckDB
trademark reply. Until then it would only run from a clone. So: after pieces 1–5, and after PyPI.

## Steps

1. Pieces 1, 2, 5 (no code change to duckOSM itself): skill, `llms.txt`, `AGENTS.md`. Test: the
   skill's recipes run as written on Monaco; `mkdocs build --strict` passes and `site/llms.txt`
   exists.
2. Pieces 3, 4: `duckosm info` and `--json`, with tests, `docs/reference/cli.md`, and the skill
   updated to use them.
3. Piece 6, after the PyPI decision: its own design note.

## Decisions for Kaveh

1. **Scope now.** Recommended: steps 1 and 2; the MCP server waits for PyPI.
2. **`AGENTS.md` and `CLAUDE.md`.** Recommended: `AGENTS.md` holds the rules, `CLAUDE.md` points to
   it. Alternative: keep both full (they will drift).
3. **Claude plugin.** Recommended: yes, a `.claude-plugin/` manifest with the skill (and the MCP
   server later), like roadstyle.
