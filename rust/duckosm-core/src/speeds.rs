//! Speeds and travel-time costs (`SpeedProcessor` and `CostCalculator` in duckOSM).
//! `maxspeed_kmh` replaces the raw `maxspeed` text. `cost_s` is the travel time in seconds, in 32-bit floats
//! as in the SQL.

use crate::filter::Mode;
use crate::model::Edge;

const DRIVING_DEFAULTS: [(&str, f64); 15] = [
    ("motorway", 110.0),
    ("motorway_link", 110.0),
    ("trunk", 90.0),
    ("trunk_link", 90.0),
    ("primary", 70.0),
    ("primary_link", 70.0),
    ("secondary", 60.0),
    ("secondary_link", 60.0),
    ("tertiary", 50.0),
    ("tertiary_link", 50.0),
    ("residential", 30.0),
    ("living_street", 20.0),
    ("service", 20.0),
    ("unclassified", 40.0),
    ("road", 40.0),
];
const WALKING_KMH: f64 = 5.0;
const CYCLING_KMH: f64 = 15.0;
const DEFAULT_DRIVING_KMH: f64 = 50.0;

/// `TRY_CAST(text AS DOUBLE)` of DuckDB: spaces around the number are allowed, `_` can separate digits,
/// and `inf`, `infinity` and `nan` are numbers.
pub fn try_cast_double(s: &str) -> Option<f64> {
    let t = s.trim_matches(|c: char| c == ' ' || c == '\t' || c == '\n' || c == '\r');
    if t.is_empty() {
        return None;
    }
    if t.contains('_') {
        let b = t.as_bytes();
        for (i, &c) in b.iter().enumerate() {
            if c == b'_'
                && !(i > 0
                    && i + 1 < b.len()
                    && b[i - 1].is_ascii_digit()
                    && b[i + 1].is_ascii_digit())
            {
                return None;
            }
        }
        return t.replace('_', "").parse::<f64>().ok();
    }
    t.parse::<f64>().ok()
}

/// The speed of a driving edge: the numeric `maxspeed`, else `maxspeed` in mph, else the default of the class.
pub fn driving_kmh(maxspeed: Option<&str>, highway: &str) -> f64 {
    if let Some(m) = maxspeed {
        if let Some(v) = try_cast_double(m) {
            return v;
        }
        if m.contains("mph") {
            // regexp_extract(maxspeed, '(\d+)', 1): the first run of digits
            let digits: String = m
                .chars()
                .skip_while(|c| !c.is_ascii_digit())
                .take_while(|c| c.is_ascii_digit())
                .collect();
            return digits.parse::<f64>().unwrap_or(0.0) * 1.60934;
        }
    }
    DRIVING_DEFAULTS
        .iter()
        .find(|(h, _)| *h == highway)
        .map_or(DEFAULT_DRIVING_KMH, |(_, v)| *v)
}

/// Set `maxspeed_kmh` (and `cost_s`) on the edges. The writer then leaves out the raw `maxspeed`.
pub fn apply(edges: &mut [Edge], mode: Mode, costs: bool) {
    for e in edges.iter_mut() {
        let kmh = match mode {
            Mode::Walking => WALKING_KMH,
            Mode::Cycling => {
                if e.dismount == Some(true) {
                    WALKING_KMH
                } else {
                    CYCLING_KMH
                }
            }
            Mode::Driving => driving_kmh(e.attrs.maxspeed.as_deref(), &e.attrs.highway),
        } as f32;
        e.maxspeed_kmh = Some(kmh);
        if costs {
            // (length_m / 1000.0) / maxspeed_kmh * 3600.0 in FLOAT; a speed that is not above 0 counts as 50
            let per_km = e.length_m / 1000.0f32;
            e.cost_s = Some(if kmh > 0.0 {
                per_km / kmh * 3600.0f32
            } else {
                per_km / 50.0f32 * 3600.0f32
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // TRY_CAST(x AS DOUBLE) of DuckDB 1.5.3, checked on 2026-10-21
    #[test]
    fn try_cast_like_duckdb() {
        let yes = [
            ("50", 50.0),
            ("50.5", 50.5),
            (" 50", 50.0),
            ("50 ", 50.0),
            ("1e2", 100.0),
            ("-5", -5.0),
            ("0", 0.0),
            ("+30", 30.0),
            ("30.", 30.0),
            ("1_000", 1000.0),
        ];
        for (s, v) in yes {
            assert_eq!(try_cast_double(s), Some(v), "{s:?}");
        }
        for s in [
            "30 mph", "30mph", "50 km/h", "none", "signals", "walk", "", "  ", "5.5.5", "0x1F",
            "50;60", "20 MPH", "50,5", "٥٠", "_1", "1_", "1__0",
        ] {
            assert_eq!(try_cast_double(s), None, "{s:?}");
        }
        for s in ["inf", "infinity", "-inf", "1e400"] {
            assert!(try_cast_double(s).unwrap().is_infinite(), "{s:?}");
        }
        for s in ["nan", "NaN"] {
            assert!(try_cast_double(s).unwrap().is_nan(), "{s:?}");
        }
    }

    #[test]
    fn mph_and_defaults() {
        assert_eq!(driving_kmh(Some("30 mph"), "residential"), 30.0 * 1.60934);
        assert_eq!(driving_kmh(Some("50 km/h"), "residential"), 30.0);
        assert_eq!(driving_kmh(None, "motorway"), 110.0);
        assert_eq!(driving_kmh(None, "footway"), 50.0);
        assert_eq!(driving_kmh(Some("15 Mph"), "service"), 20.0); // LIKE '%mph%' is case sensitive
    }
}
