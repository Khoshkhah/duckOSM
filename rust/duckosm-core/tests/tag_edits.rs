//! The tag edits of the page (`extra.tag_edits`) on the Monaco sample: a `sidewalk` tag on a road changes the walking type of its edges.
//! Without an edit the output is the golden one (modes.rs), so this file tests only what an edit changes.

use duckosm_core::filter::Mode;
use duckosm_core::fixes::{Elem, OsmFixes, TagEdit};
use duckosm_core::network::{build_network_with, Options};
use duckosm_core::pbf;
use std::path::PathBuf;

/// A secondary road of Monaco that the walking network has as `shared_road` (no sidewalk tag).
const WAY: i64 = 503462470;

#[test]
fn sidewalk_tag_edit_changes_the_walking_type() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/data/monaco_sample.osm.pbf");
    let mut ways = pbf::read_ways(&path).unwrap();
    let coords = pbf::read_coords(&path, &ways).unwrap();
    let opts = Options {
        keep_largest_component: false,
        cycling_dismount: true,
        merge_segments: false,
        global_junctions: false,
        connect_paths: false,
        snap_m: 10.0,
        process_speeds: false,
        calculate_costs: false,
        functional_types: true, // the walk type is a functional type
        build_graph: false,
        extract_restrictions: false,
        ways_tables: false,
    };
    let types = |ways: &[_], fixes: &OsmFixes| {
        let net = build_network_with(ways, &coords, Mode::Walking, &opts, fixes, &[]).unwrap();
        net.edges
            .iter()
            .filter(|e| e.osm_id == WAY)
            .map(|e| e.ftype)
            .collect::<Vec<_>>()
    };
    let before = types(&ways, &OsmFixes::default());
    assert!(
        !before.is_empty() && before.iter().all(|t| *t == Some("shared_road")),
        "{before:?}"
    );

    let fixes = OsmFixes {
        tag_edits: vec![TagEdit {
            elem: Elem::Way,
            id: WAY,
            set: vec![("sidewalk".into(), "both".into())],
            delete: vec![],
        }],
        ..Default::default()
    };
    fixes.apply_to_ways(&mut ways).unwrap();
    let after = types(&ways, &fixes);
    assert_eq!(after.len(), before.len());
    assert!(after.iter().all(|t| *t == Some("sidewalk")), "{after:?}");
}
