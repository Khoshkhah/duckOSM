# GMNS meso and micro networks

`duckosm gmns --meso --micro` adds two networks for simulators that work below the road level: road sections
(meso) and lane cells (micro).

| Reference and format | |
|---|---|
| Format | tables in the GMNS [DuckDB file](https://duckdb.org/docs/stable/internals/storage), in the layout of the meso and micro networks of [osm2gmns](https://github.com/jiawlu/OSM2GMNS). [GMNS](https://github.com/zephyr-data-specs/GMNS) has no meso or micro tables |
| Reference tool | [osm2gmns](https://github.com/jiawlu/OSM2GMNS) 0.7.6, compared by hand |
| Check | none yet against osm2gmns. The tests check the lengths, the link ends and the number of cells. [How each output is checked](../concepts/checks.md) |

```bash
duckosm gmns monaco.duckdb --meso --micro                      # -> monaco_gmns.duckdb with meso_driving, micro_driving
duckosm gmns monaco.duckdb --meso --meso-mode cycling          # cycling
```

## What it makes

| Network | Schema | One row is |
|---|---|---|
| meso | `meso_<mode>` | a road section (one for each link), or a connector between sections (one for each legal turn) |
| micro | `micro_<mode>` | a 7 m cell of a lane, a lane change between cells side by side, or a turn across a junction |

- **Meso:** each road becomes a section, about 6 m shorter at each end. Each legal turn becomes a connector: a curve
  from the end of the inbound section to the start of the outbound section.
- **Micro:** each lane is cut into cells of about 7 m. A lane change links two cells side by side. A turn links the
  end of an inbound lane to the start of an outbound lane.
- The ids come from `edge_id` (`M<edge_id>` for a section, `X<from edge_id>-<to edge_id>` for a connector). They stay
  the same when you build again.
- Lengths are in meters. Each link starts and ends at its nodes.

The tables and their columns, and the differences from osm2gmns:
[GMNS tables](gmns_tables.md#meso-and-micro-networks).

## Options

| Option | Does | Default |
|---|---|---|
| `--meso`, `--micro` | make the meso or micro network | off |
| `--meso-mode`, `--micro-mode` | their modes; repeat for several | `driving` |

The other options are those of [GMNS](gmns.md#options).

```python
from duckosm import to_meso, to_micro
to_meso("monaco_gmns.duckdb")                          # on a GMNS database
to_micro("monaco_gmns.duckdb", cell_length_m=7.0)
```

`duckosm gmns-viz monaco_gmns.duckdb` shows the lanes and the meso and micro networks ([See it](gmns.md#see-it)).
