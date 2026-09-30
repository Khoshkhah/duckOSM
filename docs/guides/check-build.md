# Check a build

## Validation during the build

Switch validation on in your [config file](build.md#with-a-config-file). The build then checks its
own result and fails if something is wrong:

```yaml
validation:
  enabled: true
```

It's off by default (and a build from the command line alone can't switch it on), but the
[config template](build.md#with-a-config-file) turns it on. On Monaco it prints, per mode:

```text
validate[driving] single_component: OK — 1 component(s), largest 1,940/1,940 (100.0%)
validate[driving] no_stranded_named: OK — 0 named edge(s) outside the largest component
validate[driving] unique_node_id: OK — 0 duplicate node_id(s) in nodes
validate[driving] way_length_conserved: OK — 0 deleted parallel arc(s) (way stretch missing while a kept edge bridges its endpoints)
validate[driving] layer_without_structure: WARN — 9 edge(s) with layer≠0 but no bridge/tunnel tag (OSM tagging; drawn grade-separated, no 3D deck)
```

| Check | Fails the build when |
|---|---|
| `single_component` | the network isn't one connected piece |
| `no_stranded_named` | a named road sits outside the main network |
| `unique_node_id` | two nodes share an id |
| `way_length_conserved` | part of an OSM way went missing when it was split into edges |
| `layer_without_structure` | never: a warning about OSM tagging (a road on another level with no bridge or tunnel tag) |

The first two only run when the build has a boundary: a whole region can legitimately have separate
pieces (islands, for example). Each check can be switched off; see [Configuration](../configuration.md).
Set `fail_on_error: false` to be warned instead of stopped.

## A report of the build

With `report: {enabled: true}` in the config, the build also writes a summary to `reports/` as
Markdown and HTML: counts per mode, what the road filter kept and dropped, the network pieces, and the
validation results.

## Check the geometry

In a clone of the repo, `scripts/validate_geometry.py` checks that every edge's geometry starts and
ends at its nodes, and that there are no loops or zero-length edges (driving and walking networks):

```bash
python scripts/validate_geometry.py --db monaco.duckdb
```

```text
Validating Geometry for Mode: driving
  ✓ No self-loops found.
  ✓ All edge endpoints match their topological nodes.
  ✓ No degenerate (zero-length) geometries.
...
ALL GEOMETRY CHECKS PASSED!
```

To look at one street in detail, see [Look up one OSM way](query.md#look-up-one-osm-way).
