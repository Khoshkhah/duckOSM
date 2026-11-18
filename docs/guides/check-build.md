# Check a build

With validation on, the build checks its own result and stops when a check fails.

```yaml
validation:
  enabled: true
```

Validation is off by default, and only a [config file](build.md#the-config-file) can turn it on. The template of
`duckosm init-config` turns it on.

| Check | Result when it fails |
|---|---|
| `unique_node_id` | the build stops: two nodes have the same id |
| `way_length_conserved` | the build stops: a part of an OSM way is missing after the split into edges |
| `layer_without_structure` | a warning only: a road with a `layer` tag and no `bridge` or `tunnel` tag |

Set `fail_on_error: false` to get warnings in place of a stop. The keys of each check:
[Configuration](../reference/configuration.md#validation).

With `report: {enabled: true}`, the build also writes a report to `reports/` (Markdown and HTML): the counts of each mode,
the roads that the filter kept and removed, the parts of the network, and the results of the checks.

In a clone of the repository, `scripts/validate_geometry.py` checks that each edge starts and ends at its nodes, and
that there are no loops and no edges of zero length:

```bash
python scripts/validate_geometry.py --db monaco.duckdb
```

To look at one street, use [`duckosm way`](query.md#look-up-one-osm-way).
