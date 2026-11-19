# Check a build

With validation on, the build checks its own result and stops when a check fails. Only a
[config file](build.md#the-config-file) can turn it on; the template of `duckosm init-config` does.

```yaml
validation:
  enabled: true
report:
  enabled: true      # also write a report
```

| Key under `validation:` | Checks | When it fails |
|---|---|---|
| `assert_unique_node_id` | no two nodes have the same id | the build stops |
| `assert_way_length_conserved` | no part of an OSM way is missing after the split into edges | the build stops |
| `warn_layer_without_structure` | a road with a `layer` tag has a `bridge` or `tunnel` tag | a warning only |
| `fail_on_error: false` | | warnings in place of a stop |

All checks are on when validation is on. Each key: [Configuration](../reference/configuration.md#validation).

With `report.enabled`, the build writes `reports/<name>_<time>.md` and `.html`: the counts of each mode, the roads that
the filter kept and removed, and the results of the checks.

To look at one street, use [`duckosm way`](query.md#look-up-one-osm-way).
