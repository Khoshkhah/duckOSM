//! Write the edges and nodes of duckosm-core as GeoParquet (version 1.0, WKB geometry, EPSG:4326).

use arrow_array::builder::{
    BinaryBuilder, BooleanBuilder, Float32Builder, Float64Builder, Int32Builder, Int64Builder,
    ListBuilder, MapBuilder, StringBuilder, TimestampMicrosecondBuilder,
};
use arrow_array::{ArrayRef, RecordBatch};
use arrow_schema::{DataType, Field, Fields, Schema, TimeUnit};
use duckosm_core::building_faces::BuildingFace;
use duckosm_core::bus::{Route, RouteEdge};
use duckosm_core::filter::{Mode, ModeWay};
use duckosm_core::geom::Pt;
use duckosm_core::graph::{
    GraphRow, MemberType, Relation, TurnPath, TurnPermission, TurnRestriction,
};
use duckosm_core::lanes::LaneRow;
use duckosm_core::model::{Attrs, Edge, MapRow, Node, Way};
use duckosm_core::network::Options;
use duckosm_core::raw::{RawNode, RawTags, RawWay};
use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::file::metadata::KeyValue;
use parquet::file::properties::WriterProperties;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;

const ROW_GROUP: usize = 65_536;

fn geo_metadata(geometry_type: &str) -> KeyValue {
    let json = format!(
        "{{\"version\":\"1.0.0\",\"primary_column\":\"geometry\",\"columns\":{{\"geometry\":{{\"encoding\":\"WKB\",\"geometry_types\":[\"{geometry_type}\"]}}}}}}"
    );
    KeyValue::new("geo".to_string(), json)
}

#[cfg(feature = "zstd")]
fn compression() -> Compression {
    Compression::ZSTD(parquet::basic::ZstdLevel::default())
}

#[cfg(all(not(feature = "zstd"), feature = "snap"))]
fn compression() -> Compression {
    Compression::SNAPPY
}

#[cfg(not(any(feature = "zstd", feature = "snap")))]
fn compression() -> Compression {
    Compression::UNCOMPRESSED
}

fn props(geometry_type: &str) -> WriterProperties {
    WriterProperties::builder()
        .set_compression(compression())
        .set_max_row_group_row_count(Some(ROW_GROUP))
        .set_key_value_metadata(Some(vec![geo_metadata(geometry_type)]))
        .build()
}

fn line_wkb(pts: &[Pt]) -> Vec<u8> {
    let mut b = Vec::with_capacity(9 + pts.len() * 16);
    b.push(1u8); // little endian
    b.extend_from_slice(&2u32.to_le_bytes()); // LineString
    b.extend_from_slice(&(pts.len() as u32).to_le_bytes());
    for p in pts {
        b.extend_from_slice(&p.0.to_le_bytes());
        b.extend_from_slice(&p.1.to_le_bytes());
    }
    b
}

fn point_wkb(p: Pt) -> Vec<u8> {
    let mut b = Vec::with_capacity(21);
    b.push(1u8);
    b.extend_from_slice(&1u32.to_le_bytes());
    b.extend_from_slice(&p.0.to_le_bytes());
    b.extend_from_slice(&p.1.to_le_bytes());
    b
}

fn list_field(name: &str) -> Field {
    Field::new(
        name,
        DataType::List(Arc::new(Field::new("item", DataType::Int64, true))),
        true,
    )
}

/// Which optional columns the edge files have.
#[derive(Clone, Copy, Debug, Default)]
pub struct EdgeCols {
    /// Cycling: the `dismount` flag.
    pub dismount: bool,
    /// The speed step ran: `maxspeed_kmh` replaces the raw `maxspeed`.
    pub speeds: bool,
    /// The cost step ran: `cost_s`.
    pub costs: bool,
    /// The name of the column of `Edge::ftype`: `walk_type` or `cycle_type`.
    pub ftype: Option<&'static str>,
}

impl EdgeCols {
    /// The columns of the edge files of `mode` when the network is built with `opts`.
    pub fn of(mode: Mode, opts: &Options) -> EdgeCols {
        EdgeCols {
            dismount: mode == Mode::Cycling && opts.cycling_dismount,
            speeds: opts.process_speeds,
            costs: opts.calculate_costs,
            ftype: match mode {
                Mode::Walking if opts.functional_types => Some("walk_type"),
                Mode::Cycling if opts.functional_types => Some("cycle_type"),
                _ => None,
            },
        }
    }
}

pub fn edges_schema(cols: EdgeCols) -> Schema {
    let mut fields = vec![
        Field::new("edge_id", DataType::Int64, false),
        Field::new("edge_ref", DataType::Utf8, true),
        Field::new("source", DataType::Int64, false),
        Field::new("target", DataType::Int64, false),
        Field::new("osm_id", DataType::Int64, false),
        Field::new("highway", DataType::Utf8, true),
        Field::new("name", DataType::Utf8, true),
        Field::new("maxspeed", DataType::Utf8, true),
        Field::new("oneway", DataType::Boolean, false),
        Field::new("lanes", DataType::Int32, true),
        Field::new("surface", DataType::Utf8, true),
        Field::new("access", DataType::Utf8, true),
        Field::new("junction", DataType::Utf8, true),
        Field::new("layer", DataType::Utf8, true),
        Field::new("bridge", DataType::Utf8, true),
        Field::new("tunnel", DataType::Utf8, true),
        Field::new("service", DataType::Utf8, true),
        list_field("refs"),
        // the column order of the duckOSM tables: the geometry before is_reverse
        Field::new("geometry", DataType::Binary, false),
        Field::new("is_reverse", DataType::Boolean, false),
        Field::new("length_m", DataType::Float32, false),
    ];
    if cols.speeds {
        fields.retain(|f| f.name() != "maxspeed");
    }
    if cols.dismount {
        fields.push(Field::new("dismount", DataType::Boolean, true));
    }
    if cols.speeds {
        fields.push(Field::new("maxspeed_kmh", DataType::Float32, true));
    }
    if cols.costs {
        fields.push(Field::new("cost_s", DataType::Float32, true));
    }
    if let Some(name) = cols.ftype {
        fields.push(Field::new(name, DataType::Utf8, true));
    }
    Schema::new(fields)
}

pub fn nodes_schema() -> Schema {
    Schema::new(vec![
        Field::new("node_id", DataType::Int64, false),
        Field::new("geometry", DataType::Binary, false),
    ])
}

fn edges_batch(schema: &Arc<Schema>, rows: &[Edge], oc: EdgeCols) -> RecordBatch {
    let n = rows.len();
    let mut edge_id = Int64Builder::with_capacity(n);
    let mut edge_ref = StringBuilder::new();
    let mut source = Int64Builder::with_capacity(n);
    let mut target = Int64Builder::with_capacity(n);
    let mut osm_id = Int64Builder::with_capacity(n);
    let mut highway = StringBuilder::new();
    let mut name = StringBuilder::new();
    let mut maxspeed = StringBuilder::new();
    let mut oneway = BooleanBuilder::with_capacity(n);
    let mut lanes = Int32Builder::with_capacity(n);
    let mut surface = StringBuilder::new();
    let mut access = StringBuilder::new();
    let mut junction = StringBuilder::new();
    let mut layer = StringBuilder::new();
    let mut bridge = StringBuilder::new();
    let mut tunnel = StringBuilder::new();
    let mut service = StringBuilder::new();
    let mut refs = ListBuilder::new(Int64Builder::new());
    let mut is_reverse = BooleanBuilder::with_capacity(n);
    let mut length_m = Float32Builder::with_capacity(n);
    let mut geometry = BinaryBuilder::new();
    let mut dismount = BooleanBuilder::with_capacity(n);
    let mut kmh = Float32Builder::with_capacity(n);
    let mut cost = Float32Builder::with_capacity(n);
    let mut ftype = StringBuilder::new();
    for e in rows {
        edge_id.append_value(e.edge_id);
        edge_ref.append_option(e.edge_ref());
        source.append_value(e.source);
        target.append_value(e.target);
        osm_id.append_value(e.osm_id);
        let a = &e.attrs;
        highway.append_value(&a.highway);
        name.append_option(a.name.as_deref());
        maxspeed.append_option(a.maxspeed.as_deref());
        oneway.append_value(e.oneway);
        lanes.append_option(e.lanes);
        surface.append_option(a.surface.as_deref());
        access.append_option(e.access.as_deref());
        junction.append_option(a.junction.as_deref());
        layer.append_option(a.layer.as_deref());
        bridge.append_option(a.bridge.as_deref());
        tunnel.append_option(a.tunnel.as_deref());
        service.append_option(a.service.as_deref());
        refs.values().append_slice(&e.refs);
        refs.append(true);
        is_reverse.append_value(e.is_reverse);
        length_m.append_value(e.length_m);
        geometry.append_value(line_wkb(&e.geometry));
        dismount.append_option(e.dismount);
        kmh.append_option(e.maxspeed_kmh);
        cost.append_option(e.cost_s);
        ftype.append_option(e.ftype);
    }
    let mut cols: Vec<ArrayRef> = vec![
        Arc::new(edge_id.finish()),
        Arc::new(edge_ref.finish()),
        Arc::new(source.finish()),
        Arc::new(target.finish()),
        Arc::new(osm_id.finish()),
        Arc::new(highway.finish()),
        Arc::new(name.finish()),
        Arc::new(oneway.finish()),
        Arc::new(lanes.finish()),
        Arc::new(surface.finish()),
        Arc::new(access.finish()),
        Arc::new(junction.finish()),
        Arc::new(layer.finish()),
        Arc::new(bridge.finish()),
        Arc::new(tunnel.finish()),
        Arc::new(service.finish()),
        Arc::new(refs.finish()),
        Arc::new(geometry.finish()),
        Arc::new(is_reverse.finish()),
        Arc::new(length_m.finish()),
    ];
    if !oc.speeds {
        cols.insert(7, Arc::new(maxspeed.finish())); // after name
    }
    if oc.dismount {
        cols.push(Arc::new(dismount.finish()));
    }
    if oc.speeds {
        cols.push(Arc::new(kmh.finish()));
    }
    if oc.costs {
        cols.push(Arc::new(cost.finish()));
    }
    if oc.ftype.is_some() {
        cols.push(Arc::new(ftype.finish()));
    }
    RecordBatch::try_new(schema.clone(), cols).expect("edge columns match the schema")
}

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// Write the edges as a GeoParquet file with LineString geometry.
pub fn write_edges(path: &Path, rows: &[Edge], oc: EdgeCols) -> Result<(), String> {
    write_edges_to(File::create(path).map_err(err)?, rows, oc)
}

/// The same as [`write_edges`], to any writer (for example a `Vec<u8>` in WebAssembly).
pub fn write_edges_to<W: Write + Send>(out: W, rows: &[Edge], oc: EdgeCols) -> Result<(), String> {
    let schema = Arc::new(edges_schema(oc));
    let mut w =
        ArrowWriter::try_new(out, schema.clone(), Some(props("LineString"))).map_err(err)?;
    for chunk in rows.chunks(ROW_GROUP) {
        w.write(&edges_batch(&schema, chunk, oc)).map_err(err)?;
    }
    w.close().map_err(err)?;
    Ok(())
}

/// Write the nodes as a GeoParquet file with Point geometry.
pub fn write_nodes(path: &Path, rows: &[Node]) -> Result<(), String> {
    write_nodes_to(File::create(path).map_err(err)?, rows)
}

/// The same as [`write_nodes`], to any writer (for example a `Vec<u8>` in WebAssembly).
pub fn write_nodes_to<W: Write + Send>(out: W, rows: &[Node]) -> Result<(), String> {
    let schema = Arc::new(nodes_schema());
    let mut w = ArrowWriter::try_new(out, schema.clone(), Some(props("Point"))).map_err(err)?;
    for chunk in rows.chunks(ROW_GROUP) {
        let mut id = Int64Builder::with_capacity(chunk.len());
        let mut g = BinaryBuilder::new();
        for n in chunk {
            id.append_value(n.node_id);
            g.append_value(point_wkb((n.lon, n.lat)));
        }
        let cols: Vec<ArrayRef> = vec![Arc::new(id.finish()), Arc::new(g.finish())];
        w.write(&RecordBatch::try_new(schema.clone(), cols).map_err(err)?)
            .map_err(err)?;
    }
    w.close().map_err(err)?;
    Ok(())
}

/// Write `edge_id_map` as a plain Parquet file (no geometry).
pub fn write_edge_id_map(path: &Path, rows: &[MapRow]) -> Result<(), String> {
    write_edge_id_map_to(File::create(path).map_err(err)?, rows)
}

/// The same as [`write_edge_id_map`], to any writer (for example a `Vec<u8>` in WebAssembly).
pub fn write_edge_id_map_to<W: Write + Send>(out: W, rows: &[MapRow]) -> Result<(), String> {
    let schema = Arc::new(Schema::new(vec![
        Field::new("old_edge_id", DataType::Int64, false),
        Field::new("new_edge_id", DataType::Int64, false),
        Field::new("seq", DataType::Int32, false),
        Field::new("is_reverse", DataType::Boolean, false),
        Field::new("osm_id", DataType::Int64, false),
    ]));
    let plain = WriterProperties::builder()
        .set_compression(compression())
        .build();
    let mut w = ArrowWriter::try_new(out, schema.clone(), Some(plain)).map_err(err)?;
    for chunk in rows.chunks(ROW_GROUP) {
        let mut old = Int64Builder::with_capacity(chunk.len());
        let mut new = Int64Builder::with_capacity(chunk.len());
        let mut seq = Int32Builder::with_capacity(chunk.len());
        let mut rev = BooleanBuilder::with_capacity(chunk.len());
        let mut osm = Int64Builder::with_capacity(chunk.len());
        for m in chunk {
            old.append_value(m.old_edge_id);
            new.append_value(m.new_edge_id);
            seq.append_value(m.seq);
            rev.append_value(m.is_reverse);
            osm.append_value(m.osm_id);
        }
        let cols: Vec<ArrayRef> = vec![
            Arc::new(old.finish()),
            Arc::new(new.finish()),
            Arc::new(seq.finish()),
            Arc::new(rev.finish()),
            Arc::new(osm.finish()),
        ];
        w.write(&RecordBatch::try_new(schema.clone(), cols).map_err(err)?)
            .map_err(err)?;
    }
    w.close().map_err(err)?;
    Ok(())
}

/// Write the rows of a plain table (no geometry), in row groups of `ROW_GROUP`.
fn write_rows<W: Write + Send, T>(
    out: W,
    fields: Vec<Field>,
    rows: &[T],
    columns: impl Fn(&[T]) -> Vec<ArrayRef>,
) -> Result<(), String> {
    let schema = Arc::new(Schema::new(fields));
    let plain = WriterProperties::builder()
        .set_compression(compression())
        .build();
    let mut w = ArrowWriter::try_new(out, schema.clone(), Some(plain)).map_err(err)?;
    for chunk in rows.chunks(ROW_GROUP) {
        w.write(&RecordBatch::try_new(schema.clone(), columns(chunk)).map_err(err)?)
            .map_err(err)?;
    }
    w.close().map_err(err)?;
    Ok(())
}

fn i64s(rows: impl Iterator<Item = i64>) -> ArrayRef {
    let mut b = Int64Builder::new();
    rows.for_each(|v| b.append_value(v));
    Arc::new(b.finish())
}

fn texts<'a>(rows: impl Iterator<Item = Option<&'a str>>) -> ArrayRef {
    let mut b = StringBuilder::new();
    rows.for_each(|v| b.append_option(v));
    Arc::new(b.finish())
}

fn field(name: &str, t: DataType) -> Field {
    Field::new(name, t, true)
}

/// Write `edge_graph` (`via_edge` is `to_edge`).
pub fn write_edge_graph_to<W: Write + Send>(out: W, rows: &[GraphRow]) -> Result<(), String> {
    let fields = vec![
        field("from_edge", DataType::Int64),
        field("to_edge", DataType::Int64),
        field("via_edge", DataType::Int64),
        field("cost", DataType::Float32),
        field("uses", DataType::Utf8),
    ];
    write_rows(out, fields, rows, |c| {
        let mut cost = Float32Builder::with_capacity(c.len());
        c.iter().for_each(|r| cost.append_option(r.cost));
        vec![
            i64s(c.iter().map(|r| r.from_edge)),
            i64s(c.iter().map(|r| r.to_edge)),
            i64s(c.iter().map(|r| r.to_edge)),
            Arc::new(cost.finish()),
            texts(c.iter().map(|r| Some(r.uses))),
        ]
    })
}

/// Write `turn_restrictions`.
pub fn write_turn_restrictions_to<W: Write + Send>(
    out: W,
    rows: &[TurnRestriction],
) -> Result<(), String> {
    let fields = vec![
        field("restriction_id", DataType::Int64),
        field("restriction_type", DataType::Utf8),
        field("except_vehicles", DataType::Utf8),
        field("applies_to", DataType::Utf8),
        field("condition", DataType::Utf8),
        field("via_node", DataType::Int64),
        field("from_edge_id", DataType::Int64),
        field("to_edge_id", DataType::Int64),
    ];
    write_rows(out, fields, rows, |c| {
        vec![
            i64s(c.iter().map(|r| r.restriction_id)),
            texts(c.iter().map(|r| Some(r.restriction_type.as_str()))),
            texts(c.iter().map(|r| r.except_vehicles.as_deref())),
            texts(c.iter().map(|r| r.applies_to.as_deref())),
            texts(c.iter().map(|r| r.condition.as_deref())),
            i64s(c.iter().map(|r| r.via_node)),
            i64s(c.iter().map(|r| r.from_edge_id)),
            i64s(c.iter().map(|r| r.to_edge_id)),
        ]
    })
}

/// Write `turn_permission`.
pub fn write_turn_permission_to<W: Write + Send>(
    out: W,
    rows: &[TurnPermission],
) -> Result<(), String> {
    let fields = vec![
        field("from_edge", DataType::Int64),
        field("to_edge", DataType::Int64),
        field("allowed", DataType::Boolean),
        field("vehicles", DataType::Utf8),
        field("except_vehicles", DataType::Utf8),
        field("condition", DataType::Utf8),
        field("restriction_id", DataType::Int64),
    ];
    write_rows(out, fields, rows, |c| {
        let mut allowed = BooleanBuilder::with_capacity(c.len());
        c.iter().for_each(|r| allowed.append_value(r.allowed));
        vec![
            i64s(c.iter().map(|r| r.from_edge)),
            i64s(c.iter().map(|r| r.to_edge)),
            Arc::new(allowed.finish()),
            texts(c.iter().map(|r| r.vehicles.as_deref())),
            texts(c.iter().map(|r| r.except_vehicles.as_deref())),
            texts(c.iter().map(|r| r.condition.as_deref())),
            i64s(c.iter().map(|r| r.restriction_id)),
        ]
    })
}

/// Write `turn_path_restrictions` (`via_edges` is a list).
pub fn write_turn_path_restrictions_to<W: Write + Send>(
    out: W,
    rows: &[TurnPath],
) -> Result<(), String> {
    let fields = vec![
        field("restriction_id", DataType::Int64),
        field("restriction_type", DataType::Utf8),
        field("except_vehicles", DataType::Utf8),
        field("applies_to", DataType::Utf8),
        field("condition", DataType::Utf8),
        field("from_edge", DataType::Int64),
        list_field("via_edges"),
        field("to_edge", DataType::Int64),
    ];
    write_rows(out, fields, rows, |c| {
        let mut via = ListBuilder::new(Int64Builder::new());
        for r in c {
            via.values().append_slice(&r.via_edges);
            via.append(true);
        }
        vec![
            i64s(c.iter().map(|r| r.restriction_id)),
            texts(c.iter().map(|r| Some(r.restriction_type.as_str()))),
            texts(c.iter().map(|r| r.except_vehicles.as_deref())),
            texts(c.iter().map(|r| r.applies_to.as_deref())),
            texts(c.iter().map(|r| r.condition.as_deref())),
            i64s(c.iter().map(|r| r.from_edge)),
            Arc::new(via.finish()),
            i64s(c.iter().map(|r| r.to_edge)),
        ]
    })
}

fn bools(rows: impl Iterator<Item = bool>) -> ArrayRef {
    let mut b = BooleanBuilder::new();
    rows.for_each(|v| b.append_value(v));
    Arc::new(b.finish())
}

fn i32s(rows: impl Iterator<Item = Option<i32>>) -> ArrayRef {
    let mut b = Int32Builder::new();
    rows.for_each(|v| b.append_option(v));
    Arc::new(b.finish())
}

/// The Arrow type of a `MAP(VARCHAR, VARCHAR)` column, as `MapBuilder` makes it.
fn map_type() -> DataType {
    let entries = Fields::from(vec![
        Field::new("key", DataType::Utf8, false),
        Field::new("value", DataType::Utf8, true),
    ]);
    DataType::Map(
        Arc::new(Field::new("entries", DataType::Struct(entries), false)),
        false,
    )
}

/// Write `<mode>.ways` (duckOSM `RoadFilter`): the ways of the mode after the road filter and the fixes,
/// with all their OSM tags (`all`: the ways as read, sorted by id, where the tags are).
pub fn write_ways_to<W: Write + Send>(out: W, rows: &[ModeWay], all: &[Way]) -> Result<(), String> {
    let fields = vec![
        field("osm_id", DataType::Int64),
        field("highway", DataType::Utf8),
        field("name", DataType::Utf8),
        field("maxspeed", DataType::Utf8),
        field("oneway", DataType::Boolean),
        field("bus_back", DataType::Boolean),
        field("surface", DataType::Utf8),
        field("service", DataType::Utf8),
        field("access", DataType::Utf8),
        field("junction", DataType::Utf8),
        field("layer", DataType::Utf8),
        field("bridge", DataType::Utf8),
        field("tunnel", DataType::Utf8),
        field("lanes_fwd", DataType::Int32),
        field("lanes_bwd", DataType::Int32),
        field("tags", map_type()),
        list_field("refs"),
    ];
    write_rows(out, fields, rows, |c| {
        let mut tags = MapBuilder::new(None, StringBuilder::new(), StringBuilder::new());
        let mut refs = ListBuilder::new(Int64Builder::new());
        for r in c {
            if let Ok(i) = all.binary_search_by_key(&r.id, |w| w.id) {
                for (k, v) in all[i].all_tags() {
                    tags.keys().append_value(k);
                    tags.values().append_value(v);
                }
            }
            tags.append(true).expect("a map of strings");
            refs.values().append_slice(&r.refs);
            refs.append(true);
        }
        let a = |f: fn(&Attrs) -> Option<&str>| texts(c.iter().map(move |r| f(&r.attrs)));
        vec![
            i64s(c.iter().map(|r| r.id)),
            texts(c.iter().map(|r| Some(r.attrs.highway.as_str()))),
            a(|x| x.name.as_deref()),
            a(|x| x.maxspeed.as_deref()),
            bools(c.iter().map(|r| r.oneway)),
            bools(c.iter().map(|r| r.bus_back)),
            a(|x| x.surface.as_deref()),
            a(|x| x.service.as_deref()),
            texts(c.iter().map(|r| r.access.as_deref())),
            a(|x| x.junction.as_deref()),
            a(|x| x.layer.as_deref()),
            a(|x| x.bridge.as_deref()),
            a(|x| x.tunnel.as_deref()),
            i32s(c.iter().map(|r| r.lanes_fwd)),
            i32s(c.iter().map(|r| r.lanes_bwd)),
            Arc::new(tags.finish()),
            Arc::new(refs.finish()),
        ]
    })
}

/// Write `<mode>.way_nodes`: the nodes of `ways`, one row per node (`seq` from 0).
pub fn write_way_nodes_to<W: Write + Send>(out: W, rows: &[ModeWay]) -> Result<(), String> {
    let flat: Vec<(i64, i64, i64)> = rows
        .iter()
        .flat_map(|w| {
            w.refs
                .iter()
                .enumerate()
                .map(move |(i, &n)| (w.id, n, i as i64))
        })
        .collect();
    let fields = vec![
        field("way_id", DataType::Int64),
        field("node_id", DataType::Int64),
        field("seq", DataType::Int64),
    ];
    write_rows(out, fields, &flat, |c| {
        vec![
            i64s(c.iter().map(|r| r.0)),
            i64s(c.iter().map(|r| r.1)),
            i64s(c.iter().map(|r| r.2)),
        ]
    })
}

/// Write `main.global_junctions` (node id, is a road junction), sorted by node id.
pub fn write_global_junctions_to<W: Write + Send>(
    out: W,
    rows: &[(i64, bool)],
) -> Result<(), String> {
    let fields = vec![
        field("node_id", DataType::Int64),
        field("is_road_junction", DataType::Boolean),
    ];
    write_rows(out, fields, rows, |c| {
        vec![i64s(c.iter().map(|r| r.0)), bools(c.iter().map(|r| r.1))]
    })
}

/// A row of `mm.transfers`: a change of mode at a node that walking shares with another mode.
pub struct Transfer {
    pub node_id: i64,
    pub from_mode: &'static str,
    pub to_mode: &'static str,
    pub cost_s: f64,
    pub kind: &'static str,
}

/// Write `mm.transfers`.
pub fn write_transfers_to<W: Write + Send>(out: W, rows: &[Transfer]) -> Result<(), String> {
    let fields = vec![
        field("node_id", DataType::Int64),
        field("from_mode", DataType::Utf8),
        field("to_mode", DataType::Utf8),
        field("cost_s", DataType::Float64),
        field("kind", DataType::Utf8),
    ];
    write_rows(out, fields, rows, |c| {
        let mut cost = Float64Builder::new();
        c.iter().for_each(|r| cost.append_value(r.cost_s));
        vec![
            i64s(c.iter().map(|r| r.node_id)),
            texts(c.iter().map(|r| Some(r.from_mode))),
            texts(c.iter().map(|r| Some(r.to_mode))),
            Arc::new(cost.finish()),
            texts(c.iter().map(|r| Some(r.kind))),
        ]
    })
}

/// Write `driving.lane_profile`.
pub fn write_lane_profile_to<W: Write + Send>(out: W, rows: &[LaneRow]) -> Result<(), String> {
    let fields = vec![
        field("edge_id", DataType::Int64),
        field("lane_num", DataType::Int32),
        field("use", DataType::Utf8),
        field("width_m", DataType::Float64),
        field("turn", DataType::Utf8),
        field("source", DataType::Utf8),
        field("width_source", DataType::Utf8),
    ];
    write_rows(out, fields, rows, |c| {
        let mut width = Float64Builder::new();
        c.iter().for_each(|r| width.append_value(r.width_m));
        vec![
            i64s(c.iter().map(|r| r.edge_id)),
            i32s(c.iter().map(|r| Some(r.lane_num))),
            texts(c.iter().map(|r| Some(r.use_))),
            Arc::new(width.finish()),
            texts(c.iter().map(|r| r.turn.as_deref())),
            texts(c.iter().map(|r| Some(r.source.as_str()))),
            texts(c.iter().map(|r| Some(r.width_source.as_str()))),
        ]
    })
}

/// A property of the boundary file, typed as DuckDB's `ST_Read` (GDAL) types it.
pub enum Prop {
    Int(i32),
    BigInt(i64),
    Double(f64),
    Bool(bool),
    Text(Option<String>),
}

/// The WKB of a Polygon (one polygon) or a MultiPolygon: polygons of rings of points.
pub fn polygons_wkb(polys: &[Vec<Vec<Pt>>], multi: bool) -> Vec<u8> {
    fn polygon(b: &mut Vec<u8>, rings: &[Vec<Pt>]) {
        b.push(1u8);
        b.extend_from_slice(&3u32.to_le_bytes());
        b.extend_from_slice(&(rings.len() as u32).to_le_bytes());
        for r in rings {
            b.extend_from_slice(&(r.len() as u32).to_le_bytes());
            for p in r {
                b.extend_from_slice(&p.0.to_le_bytes());
                b.extend_from_slice(&p.1.to_le_bytes());
            }
        }
    }
    let mut b = Vec::new();
    if multi {
        b.push(1u8);
        b.extend_from_slice(&6u32.to_le_bytes());
        b.extend_from_slice(&(polys.len() as u32).to_le_bytes());
        polys.iter().for_each(|p| polygon(&mut b, p));
    } else {
        polygon(&mut b, &polys[0]);
    }
    b
}

/// Write `main.boundary`: one row with `OGC_FID` (0), the properties of the boundary file, and the shape in
/// `geometry` (WKB, a Polygon or a MultiPolygon).
pub fn write_boundary_to<W: Write + Send>(
    out: W,
    columns: &[(String, Prop)],
    wkb: &[u8],
    multi: bool,
) -> Result<(), String> {
    let mut fields = vec![field("OGC_FID", DataType::Int64)];
    let mut cols: Vec<ArrayRef> = vec![i64s(std::iter::once(0))];
    for (name, p) in columns {
        let (t, col): (DataType, ArrayRef) = match p {
            Prop::Int(v) => (DataType::Int32, i32s(std::iter::once(Some(*v)))),
            Prop::BigInt(v) => (DataType::Int64, i64s(std::iter::once(*v))),
            Prop::Double(v) => {
                let mut b = Float64Builder::new();
                b.append_value(*v);
                (DataType::Float64, Arc::new(b.finish()))
            }
            Prop::Bool(v) => (DataType::Boolean, bools(std::iter::once(*v))),
            Prop::Text(v) => (DataType::Utf8, texts(std::iter::once(v.as_deref()))),
        };
        fields.push(field(name, t));
        cols.push(col);
    }
    fields.push(field("geometry", DataType::Binary));
    let mut g = BinaryBuilder::new();
    g.append_value(wkb);
    cols.push(Arc::new(g.finish()));
    let schema = Arc::new(Schema::new(fields));
    let kind = if multi { "MultiPolygon" } else { "Polygon" };
    let mut w = ArrowWriter::try_new(out, schema.clone(), Some(props(kind))).map_err(err)?;
    w.write(&RecordBatch::try_new(schema, cols).map_err(err)?)
        .map_err(err)?;
    w.close().map_err(err)?;
    Ok(())
}

/// The row of `main.visualization_metadata`.
pub struct Metadata {
    /// The boundary as GeoJSON text (the `JSON` column).
    pub boundary_geojson: String,
    pub center_lat: f64,
    pub center_lon: f64,
    pub initial_zoom: i32,
    pub timezone: Option<String>,
    /// Microseconds since 1970 (UTC).
    pub built_at: i64,
    pub version: String,
}

/// Write `main.visualization_metadata` (one row).
pub fn write_metadata_to<W: Write + Send>(out: W, m: &Metadata) -> Result<(), String> {
    let fields = vec![
        field("boundary_geojson", DataType::Utf8),
        field("center_lat", DataType::Float64),
        field("center_lon", DataType::Float64),
        field("initial_zoom", DataType::Int32),
        field("timezone", DataType::Utf8),
        field("built_at", DataType::Timestamp(TimeUnit::Microsecond, None)),
        field("duckosm_version", DataType::Utf8),
    ];
    write_rows(out, fields, std::slice::from_ref(m), |c| {
        let mut lat = Float64Builder::new();
        let mut lon = Float64Builder::new();
        let mut at = TimestampMicrosecondBuilder::new();
        for r in c {
            lat.append_value(r.center_lat);
            lon.append_value(r.center_lon);
            at.append_value(r.built_at);
        }
        vec![
            texts(c.iter().map(|r| Some(r.boundary_geojson.as_str()))),
            Arc::new(lat.finish()),
            Arc::new(lon.finish()),
            i32s(c.iter().map(|r| Some(r.initial_zoom))),
            texts(c.iter().map(|r| r.timezone.as_deref())),
            Arc::new(at.finish()),
            texts(c.iter().map(|r| Some(r.version.as_str()))),
        ]
    })
}

/// Write rows as a plain Parquet file, one row group for each `ROW_GROUP` rows: `batch` makes the columns of a part,
/// each with its name and whether it can hold NULL. The writer keeps the schema of the first part, so the flag must
/// not come from the data of a part.
fn write_parts<W: Write + Send, T>(
    out: W,
    rows: &[T],
    batch: impl Fn(&[T]) -> Vec<(&'static str, ArrayRef, bool)>,
) -> Result<(), String> {
    let part = |c: &[T]| RecordBatch::try_from_iter_with_nullable(batch(c)).map_err(err);
    let first = part(&rows[..rows.len().min(ROW_GROUP)])?;
    let plain = WriterProperties::builder()
        .set_compression(compression())
        .set_max_row_group_row_count(Some(ROW_GROUP))
        .build();
    let mut w = ArrowWriter::try_new(out, first.schema(), Some(plain)).map_err(err)?;
    w.write(&first).map_err(err)?;
    for chunk in rows.chunks(ROW_GROUP).skip(1) {
        w.write(&part(chunk)?).map_err(err)?;
    }
    w.close().map_err(err)?;
    Ok(())
}

/// The tags as a DuckDB `MAP(VARCHAR, VARCHAR)`; no tags: NULL (as `ST_READOSM`).
fn tags_column<'a>(rows: impl Iterator<Item = &'a RawTags>) -> ArrayRef {
    let mut b = MapBuilder::new(None, StringBuilder::new(), StringBuilder::new());
    for tags in rows {
        for (k, v) in tags {
            b.keys().append_value(k);
            b.values().append_value(v);
        }
        b.append(!tags.is_empty())
            .expect("the keys and the values have the same length");
    }
    Arc::new(b.finish())
}

/// `raw.nodes` of duckOSM: osm_id, lat, lon, tags.
pub fn write_raw_nodes_to<W: Write + Send>(out: W, rows: &[RawNode]) -> Result<(), String> {
    write_parts(out, rows, |c| {
        let mut id = Int64Builder::with_capacity(c.len());
        let (mut lat, mut lon) = (
            Float64Builder::with_capacity(c.len()),
            Float64Builder::with_capacity(c.len()),
        );
        for n in c {
            id.append_value(n.id);
            lat.append_value(n.lat);
            lon.append_value(n.lon);
        }
        vec![
            ("osm_id", Arc::new(id.finish()) as ArrayRef, false),
            ("lat", Arc::new(lat.finish()), false),
            ("lon", Arc::new(lon.finish()), false),
            ("tags", tags_column(c.iter().map(|n| &n.tags)), true),
        ]
    })
}

/// `raw.ways` of duckOSM: osm_id, tags, refs. The ways with fewer than 2 nodes are not in it. One more column, `line`: the
/// way as WKB from the points of its nodes in `nodes` (sorted by id), as duckOSM's `geom.way_line` makes it with
/// `ST_MakeLine` (a node that is not in `nodes` is left out; fewer than 2 points: NULL). features.sql reads it, so
/// DuckDB does not join the nodes (docs/features.md).
pub fn write_raw_ways_to<W: Write + Send>(
    out: W,
    rows: &[RawWay],
    nodes: &[RawNode],
) -> Result<(), String> {
    let rows: Vec<&RawWay> = rows.iter().filter(|w| w.refs.len() >= 2).collect();
    write_parts(out, &rows, |c| {
        let mut id = Int64Builder::with_capacity(c.len());
        let mut refs = ListBuilder::new(Int64Builder::new());
        let mut line = BinaryBuilder::new();
        for w in c {
            id.append_value(w.id);
            refs.values().append_slice(&w.refs);
            refs.append(true);
            let pts: Vec<Pt> = w
                .refs
                .iter()
                .filter_map(|r| {
                    let i = nodes.binary_search_by_key(r, |n| n.id).ok()?;
                    Some((nodes[i].lon, nodes[i].lat))
                })
                .collect();
            line.append_option((pts.len() >= 2).then(|| line_wkb(&pts)));
        }
        vec![
            ("osm_id", Arc::new(id.finish()) as ArrayRef, false),
            ("tags", tags_column(c.iter().map(|w| &w.tags)), true),
            ("refs", Arc::new(refs.finish()), false),
            ("line", Arc::new(line.finish()), true),
        ]
    })
}

/// `raw.relations` of duckOSM: osm_id, tags, refs, ref_roles, ref_types (`node`, `way` or `relation`; DuckDB
/// casts the text to duckOSM's ENUM).
pub fn write_raw_relations_to<W: Write + Send>(out: W, rows: &[Relation]) -> Result<(), String> {
    write_parts(out, rows, |c| {
        let mut id = Int64Builder::with_capacity(c.len());
        let mut refs = ListBuilder::new(Int64Builder::new());
        let mut roles = ListBuilder::new(StringBuilder::new());
        let mut types = ListBuilder::new(StringBuilder::new());
        for r in c {
            id.append_value(r.id);
            for (m, t, role) in &r.members {
                refs.values().append_value(*m);
                // no role: NULL, as ST_READOSM
                roles
                    .values()
                    .append_option((!role.is_empty()).then_some(role));
                types.values().append_value(match t {
                    MemberType::Node => "node",
                    MemberType::Way => "way",
                    MemberType::Relation => "relation",
                });
            }
            refs.append(true);
            roles.append(true);
            types.append(true);
        }
        vec![
            ("osm_id", Arc::new(id.finish()) as ArrayRef, false),
            ("tags", tags_column(c.iter().map(|r| &r.tags)), true),
            ("refs", Arc::new(refs.finish()), false),
            ("ref_roles", Arc::new(roles.finish()), false),
            ("ref_types", Arc::new(types.finish()), false),
        ]
    })
}

/// Write `visualization.edge_levels` (docs/levels.md): the four numbers of each edge, and the heads and the caps.
pub fn write_edge_levels_to<W: Write + Send>(
    out: W,
    rows: &[roadstyle_levels::Row],
) -> Result<(), String> {
    let fields = vec![
        field("edge_id", DataType::Int64),
        field("casing_start", DataType::Int64),
        field("casing_level", DataType::Int64),
        field("casing_end", DataType::Int64),
        field("fill_level", DataType::Int64),
        field("head_start_m", DataType::Float64),
        field("head_end_m", DataType::Float64),
        field("cap_start", DataType::Utf8),
        field("cap_end", DataType::Utf8),
    ];
    write_rows(out, fields, rows, |c| {
        let f64s = |get: fn(&roadstyle_levels::Row) -> Option<f64>| -> ArrayRef {
            let mut b = Float64Builder::new();
            c.iter().for_each(|r| b.append_option(get(r)));
            Arc::new(b.finish())
        };
        vec![
            i64s(c.iter().map(|r| r.edge_id)),
            i64s(c.iter().map(|r| r.casing_start)),
            i64s(c.iter().map(|r| r.casing_level)),
            i64s(c.iter().map(|r| r.casing_end)),
            i64s(c.iter().map(|r| r.fill_level)),
            f64s(|r| r.head_start_m),
            f64s(|r| r.head_end_m),
            texts(c.iter().map(|r| r.cap_start)),
            texts(c.iter().map(|r| r.cap_end)),
        ]
    })
}

/// The one row of `visualization.edge_levels_meta`: where the numbers come from (`source` is `tags` or `solve`; a
/// level area of duckOSM writes `area` and its folder in `area`).
pub struct LevelsMeta {
    pub source: &'static str,
    pub n_edges: i64,
    pub edge_hash: String,
    pub roadstyle_version: &'static str,
    /// ISO 8601 text, as roadstyle writes it (`2026-10-22T00:20:45+00:00`).
    pub created: String,
}

/// Write `visualization.edge_levels_meta`.
pub fn write_edge_levels_meta_to<W: Write + Send>(out: W, m: &LevelsMeta) -> Result<(), String> {
    let fields = vec![
        field("source", DataType::Utf8),
        field("area", DataType::Utf8),
        field("n_edges", DataType::Int64),
        field("edge_hash", DataType::Utf8),
        field("roadstyle_version", DataType::Utf8),
        field("created", DataType::Utf8),
    ];
    write_rows(out, fields, std::slice::from_ref(m), |c| {
        vec![
            texts(c.iter().map(|r| Some(r.source))),
            texts(c.iter().map(|_| None)),
            i64s(c.iter().map(|r| r.n_edges)),
            texts(c.iter().map(|r| Some(r.edge_hash.as_str()))),
            texts(c.iter().map(|r| Some(r.roadstyle_version))),
            texts(c.iter().map(|r| Some(r.created.as_str()))),
        ]
    })
}

/// Write `bus.routes` (`gaps` is a list of text, VARCHAR[]).
pub fn write_routes_to<W: Write + Send>(out: W, rows: &[Route]) -> Result<(), String> {
    let fields = vec![
        field("osm_id", DataType::Int64),
        field("route", DataType::Utf8),
        field("ref", DataType::Utf8),
        field("name", DataType::Utf8),
        field("operator", DataType::Utf8),
        field("from", DataType::Utf8),
        field("to", DataType::Utf8),
        field("network", DataType::Utf8),
        field("edges", DataType::Int32),
        field("outside", DataType::Int32),
        field(
            "gaps",
            DataType::List(Arc::new(Field::new("item", DataType::Utf8, true))),
        ),
    ];
    write_rows(out, fields, rows, |c| {
        let mut gaps = ListBuilder::new(StringBuilder::new());
        for r in c {
            r.gaps.iter().for_each(|g| gaps.values().append_value(g));
            gaps.append(true);
        }
        vec![
            i64s(c.iter().map(|r| r.osm_id)),
            texts(c.iter().map(|r| r.route.as_deref())),
            texts(c.iter().map(|r| r.ref_.as_deref())),
            texts(c.iter().map(|r| r.name.as_deref())),
            texts(c.iter().map(|r| r.operator.as_deref())),
            texts(c.iter().map(|r| r.from.as_deref())),
            texts(c.iter().map(|r| r.to.as_deref())),
            texts(c.iter().map(|r| r.network.as_deref())),
            i32s(c.iter().map(|r| Some(r.edges))),
            i32s(c.iter().map(|r| Some(r.outside))),
            Arc::new(gaps.finish()),
        ]
    })
}

/// Write `bus.route_edges`.
pub fn write_route_edges_to<W: Write + Send>(out: W, rows: &[RouteEdge]) -> Result<(), String> {
    let fields = vec![
        field("route_id", DataType::Int64),
        field("ref", DataType::Utf8),
        field("seq", DataType::Int32),
        field("edge_id", DataType::Int64),
        field("edge_ref", DataType::Utf8),
        field("bus_lane", DataType::Boolean),
    ];
    write_rows(out, fields, rows, |c| {
        vec![
            i64s(c.iter().map(|r| r.route_id)),
            texts(c.iter().map(|r| r.ref_.as_deref())),
            i32s(c.iter().map(|r| Some(r.seq))),
            i64s(c.iter().map(|r| r.edge_id)),
            texts(c.iter().map(|r| r.edge_ref.as_deref())),
            bools(c.iter().map(|r| r.bus_lane)),
        ]
    })
}

/// The columns of `main.building_faces` (`building_faces_main.parquet`): one row for each wall of a building (`duckosm_core::building_faces`).
pub fn building_faces_schema() -> Schema {
    Schema::new(vec![
        Field::new("building_id", DataType::Int64, false),
        Field::new("building_type", DataType::Utf8, false),
        Field::new("face_index", DataType::Int32, false),
        Field::new("from_node", DataType::Int64, false),
        Field::new("to_node", DataType::Int64, false),
        Field::new("face_type", DataType::Utf8, false),
        Field::new("computed_type", DataType::Utf8, false),
        Field::new("edited", DataType::Boolean, false),
        Field::new("type_reason", DataType::Utf8, false),
        Field::new("door", DataType::Utf8, true),
        Field::new("length_m", DataType::Float32, false),
        Field::new("azimuth_deg", DataType::Float32, false),
        Field::new("street_osm_id", DataType::Int64, true),
        Field::new("street_edge_id", DataType::Int64, true),
        Field::new("street_name", DataType::Utf8, true),
        Field::new("distance_m", DataType::Float32, true),
        Field::new("geometry", DataType::Binary, false),
    ])
}

/// Write `building_faces_main.parquet` as GeoParquet with a LineString for each wall.
pub fn write_building_faces_to<W: Write + Send>(out: W, rows: &[BuildingFace]) -> Result<(), String> {
    let schema = Arc::new(building_faces_schema());
    let mut w = ArrowWriter::try_new(out, schema.clone(), Some(props("LineString"))).map_err(err)?;
    for chunk in rows.chunks(ROW_GROUP) {
        let n = chunk.len();
        let (mut building_id, mut from_node, mut to_node) = (Int64Builder::with_capacity(n), Int64Builder::with_capacity(n), Int64Builder::with_capacity(n));
        let mut face_index = Int32Builder::with_capacity(n);
        let (mut face_type, mut computed_type, mut street_name) = (StringBuilder::new(), StringBuilder::new(), StringBuilder::new());
        let mut edited = BooleanBuilder::with_capacity(n);
        let (mut door, mut type_reason, mut building_type) = (StringBuilder::new(), StringBuilder::new(), StringBuilder::new());
        let (mut length_m, mut azimuth_deg, mut distance_m) = (Float32Builder::with_capacity(n), Float32Builder::with_capacity(n), Float32Builder::with_capacity(n));
        let (mut street_osm_id, mut street_edge_id) = (Int64Builder::with_capacity(n), Int64Builder::with_capacity(n));
        let mut geometry = BinaryBuilder::new();
        for r in chunk {
            building_id.append_value(r.building_id);
            face_index.append_value(r.face_index);
            from_node.append_value(r.from_node);
            to_node.append_value(r.to_node);
            face_type.append_value(r.face_type.name());
            computed_type.append_value(r.computed_type.name());
            edited.append_value(r.edited);
            type_reason.append_value(r.type_reason);
            building_type.append_value(r.building_type);
            door.append_option(r.door.as_deref());
            length_m.append_value(r.length_m);
            azimuth_deg.append_value(r.azimuth_deg);
            street_osm_id.append_option(r.street_osm_id);
            street_edge_id.append_option(r.street_edge_id);
            street_name.append_option(r.street_name.as_deref());
            distance_m.append_option(r.distance_m);
            geometry.append_value(line_wkb(&r.geometry));
        }
        let cols: Vec<ArrayRef> = vec![
            Arc::new(building_id.finish()),
            Arc::new(building_type.finish()),
            Arc::new(face_index.finish()),
            Arc::new(from_node.finish()),
            Arc::new(to_node.finish()),
            Arc::new(face_type.finish()),
            Arc::new(computed_type.finish()),
            Arc::new(edited.finish()),
            Arc::new(type_reason.finish()),
            Arc::new(door.finish()),
            Arc::new(length_m.finish()),
            Arc::new(azimuth_deg.finish()),
            Arc::new(street_osm_id.finish()),
            Arc::new(street_edge_id.finish()),
            Arc::new(street_name.finish()),
            Arc::new(distance_m.finish()),
            Arc::new(geometry.finish()),
        ];
        w.write(&RecordBatch::try_new(schema.clone(), cols).map_err(err)?).map_err(err)?;
    }
    w.close().map_err(err)?;
    Ok(())
}

/// Write the heights of the nodes of a mode (`elevation_nodes_<mode>.parquet`): `node_id`, `ele`.
pub fn write_node_heights_to<W: Write + Send>(out: W, rows: &[(i64, f64)]) -> Result<(), String> {
    let fields = vec![
        field("node_id", DataType::Int64),
        field("ele", DataType::Float64),
    ];
    write_rows(out, fields, rows, |c| {
        let mut ele = Float64Builder::new();
        c.iter().for_each(|r| ele.append_value(r.1));
        vec![i64s(c.iter().map(|r| r.0)), Arc::new(ele.finish())]
    })
}

/// Write the heights of the edges of a mode (`elevation_edges_<mode>.parquet`): `edge_id`, `z_from`, `z_to`,
/// `z_vertices` (one height for each vertex of the geometry, in its order).
pub fn write_edge_heights_to<W: Write + Send>(
    out: W,
    rows: &[(i64, f64, f64, Vec<f64>)],
) -> Result<(), String> {
    let fields = vec![
        field("edge_id", DataType::Int64),
        field("z_from", DataType::Float64),
        field("z_to", DataType::Float64),
        field(
            "z_vertices",
            DataType::List(Arc::new(Field::new("item", DataType::Float64, true))),
        ),
    ];
    write_rows(out, fields, rows, |c| {
        let (mut zf, mut zt) = (Float64Builder::new(), Float64Builder::new());
        let mut zv = ListBuilder::new(Float64Builder::new());
        for r in c {
            zf.append_value(r.1);
            zt.append_value(r.2);
            r.3.iter().for_each(|z| zv.values().append_value(*z));
            zv.append(true);
        }
        vec![
            i64s(c.iter().map(|r| r.0)),
            Arc::new(zf.finish()),
            Arc::new(zt.finish()),
            Arc::new(zv.finish()),
        ]
    })
}

/// Write duckOSM's `main.elevation_metadata` (`elevation_metadata_main.parquet`), one row for the column `ele`:
/// the text columns in duckOSM's order (ele_column, source, source_type, uri, product, dem_crs, vertical_datum,
/// license, sampled_at, duckosm_version), the resolution, the fill value and the two counts.
pub fn write_elevation_metadata_to<W: Write + Send>(
    out: W,
    text: [&str; 10],
    resolution_m: f64,
    nodata_fill: f64,
    n_nodes: i64,
    n_nodata: i64,
) -> Result<(), String> {
    let names = [
        "ele_column",
        "source",
        "source_type",
        "uri",
        "resolution_m",
        "product",
        "dem_crs",
        "vertical_datum",
        "license",
        "nodata_fill",
        "n_nodes",
        "n_nodata",
        "sampled_at",
        "duckosm_version",
    ];
    let fields = names
        .iter()
        .map(|n| {
            field(
                n,
                match *n {
                    "resolution_m" | "nodata_fill" => DataType::Float64,
                    "n_nodes" | "n_nodata" => DataType::Int64,
                    _ => DataType::Utf8,
                },
            )
        })
        .collect();
    write_rows(out, fields, &[()], |_| {
        let t = |i: usize| texts(std::iter::once(Some(text[i])));
        let f = |v: f64| -> ArrayRef { Arc::new(arrow_array::Float64Array::from(vec![v])) };
        vec![
            t(0),
            t(1),
            t(2),
            t(3),
            f(resolution_m),
            t(4),
            t(5),
            t(6),
            t(7),
            f(nodata_fill),
            i64s(std::iter::once(n_nodes)),
            i64s(std::iter::once(n_nodata)),
            t(8),
            t(9),
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

    /// A NULL only in the second part (row 65,537) must fit the schema of the first part.
    #[test]
    fn null_after_the_first_part() {
        let tag = vec![("highway".to_string(), "crossing".to_string())];
        let mut rows: Vec<RawNode> = (0..=ROW_GROUP as i64)
            .map(|id| RawNode {
                id,
                lon: 0.0,
                lat: 0.0,
                tags: tag.clone(),
            })
            .collect();
        rows.last_mut().unwrap().tags.clear();
        let path =
            std::env::temp_dir().join(format!("duckosm-null-{}.parquet", std::process::id()));
        write_raw_nodes_to(std::fs::File::create(&path).unwrap(), &rows).unwrap();
        let reader = ParquetRecordBatchReaderBuilder::try_new(std::fs::File::open(&path).unwrap())
            .unwrap()
            .build()
            .unwrap();
        let nulls: usize = reader
            .map(|b| b.unwrap().column_by_name("tags").unwrap().null_count())
            .sum();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(nulls, 1);
    }
}
