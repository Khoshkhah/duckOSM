# railML export → `duckosm railml`

`duckosm railml` (`to_railml`): rail infrastructure in railML 2.4. A single open **railML
infrastructure** file feeds **OpenTrack, RailSys, FBS, Viriato, OpenTimeTable**.

## The scope reality (read first)

Unlike GMNS / MATSim / OpenDRIVE — which all **reuse duckOSM's road network** — railML has **no
existing substrate to reuse**: duckOSM builds *road* modes (driving/walking/cycling), not a rail
network. So this export needs a **new rail extraction** (railway ways → a track network with topology,
switches, signals, stations). That's meaningfully more work than the road-reusing exports, and the
honest reason it sat last on the roadmap.

**But the data is there.** Södermalm's raw OSM (in `raw.ways`/`raw.nodes`) has **33 `rail` + 26
`subway` ways, 30 switches, 68 signals, 7 stations, 2 buffer stops** — enough for a real railML
infrastructure. (A county build would have far more.)

## Target: railML 2.4 `<infrastructure>`

railML **2.4** (not 3.x) — the widest tool support (OpenTrack/RailSys import it), and a track-based
model that maps cleanly from OSM ways. Scope is **infrastructure only** (no timetable / rollingstock).

```xml
<railml xmlns="https://www.railml.org/schemas/2013" version="2.4">
  <infrastructure id="is1">
    <operationControlPoints>
      <ocp id="ocp_{node}" name="Slussen"><geoCoord coord="59.319 18.072"/></ocp>
    </operationControlPoints>
    <tracks>
      <track id="tr_{eid}" type="mainTrack">
        <trackTopology>
          <trackBegin id="..." pos="0"><connection id="..." ref="..."/></trackBegin>
          <trackEnd  id="..." pos="123.4"><bufferStop id="..."/></trackEnd>
        </trackTopology>
        <trackElements><switches><switch id="sw_{node}" pos="..."/></switches></trackElements>
        <ocsElements><signals><signal id="sig_{node}" pos="..." dir="up"/></signals></ocsElements>
      </track>
    </tracks>
  </infrastructure>
</railml>
```

## Rail extraction (the new part)

1. Pull `railway IN (rail, light_rail, subway, tram, narrow_gauge, funicular)` ways from `raw.ways`
   (+ their node refs) — reusing the **same way-splitting logic the road builder uses**: split each
   way at nodes shared with another rail way or at a `railway=switch` node → **track segments**.
2. **Tracks** — one `<track>` per segment: `length` from the geometry; `<trackBegin>`/`<trackEnd>` with
   `<connection>` to the adjacent track(s) at each end (topology), or `<bufferStop>` where a
   `railway=buffer_stop` node sits or the end is free.
3. **Switches** — `railway=switch` nodes → `<switch>` in the owning track's `trackElements`, `pos` =
   mileage along the track; connections wire the diverging tracks.
4. **Signals** — `railway=signal` nodes → `<signal>` in `ocsElements` at their `pos`.
5. **OCPs** — `railway=station`/`halt` nodes (and `public_transport=station`) → `<ocp>` with a
   `<geoCoord>`; tracks reference the ocp they pass through.

## Mapping: OSM → railML

| railML | ← OSM |
|---|---|
| `track` | a rail-way segment (split at switches/junctions); `id = tr_{edge-hash}` |
| `track/@length` | segment geometry length (m) |
| `trackBegin/End` `connection` | shared endpoint with the adjacent segment (topology) |
| `switch` | `railway=switch` node, `pos` along track |
| `signal` | `railway=signal` node, `pos` + direction |
| `ocp` | `railway=station`/`halt` (name, geoCoord) |
| `bufferStop` | `railway=buffer_stop` node / free end |

## CLI / API

```bash
duckosm railml sodermalm_pbf.duckdb                        # -> sodermalm_pbf.railml.xml (railML 2.4)
```
`to_railml(source_db, out_path, rail_types=(...))` → counts `{tracks, switches, signals, ocps}`.
Reads the built db's `raw` schema (railway ways/nodes), so no re-parse of the PBF.

## Fidelity & honesty

- OSM rail **topology is partial** — track connectivity is inferred from shared nodes, which is good
  for main lines but misses platform-track detail and exact switch geometry.
- **Electrification, gauge, speed, gradient** are sparsely tagged in OSM → mostly omitted / defaulted.
- **Infrastructure only** — no timetable, rollingstock, or interlocking.
- **Topology simplification:** connections are wired 1:1 by pairing the track-ends at each node
  (clean at degree-2 joins); nodes with ≥3 ends are marked with a `<switch>` and paired as best they
  can — a proper facing-point switch (through + branch connections) is a refinement.
- **Validation:** the railML 2.4 XSD isn't freely fetchable (railml.org requires registration), so —
  like OpenDRIVE — validation is **structural** (well-formed + every `connection/@ref` resolves to a
  real `connection/@id` + every track has begin/end) with a **railVIVID / OpenTrack load** as the
  manual acceptance check. No local validator here, so simulator acceptance is unproven — flagged.

## Delivered

**v1 = full infrastructure** (tracks + topology + switches + signals + OCPs + buffer stops), railML
2.4 — `to_railml(source_db, out_path)` / `duckosm railml`. `tests/test_railml.py` covers the counts,
topology cross-refs, switch/signal placement, and the empty/missing-raw cases.
