//! JSON as Python's `json.dumps(..., indent=1)` writes it, and DuckDB values as the Python package of DuckDB gives them.

use duckdb::types::{TimeUnit, Value};
use serde_json::{json, Value as J};

/// `json.dumps(v, indent=1)`: one space for each level, `": "` after a key, floats as Python's `repr`, and non-ASCII
/// letters as `\uXXXX`.
pub fn dumps(v: &J) -> String {
    let mut out = String::new();
    write(v, 0, &mut out);
    out
}

fn write(v: &J, level: usize, out: &mut String) {
    match v {
        J::Array(a) if a.is_empty() => out.push_str("[]"),
        J::Object(o) if o.is_empty() => out.push_str("{}"),
        J::Array(a) => {
            out.push_str("[\n");
            for (i, x) in a.iter().enumerate() {
                out.push_str(&" ".repeat(level + 1));
                write(x, level + 1, out);
                out.push_str(if i + 1 < a.len() { ",\n" } else { "\n" });
            }
            out.push_str(&" ".repeat(level));
            out.push(']');
        }
        J::Object(o) => {
            out.push_str("{\n");
            for (i, (k, x)) in o.iter().enumerate() {
                out.push_str(&" ".repeat(level + 1));
                out.push_str(&text(k));
                out.push_str(": ");
                write(x, level + 1, out);
                out.push_str(if i + 1 < o.len() { ",\n" } else { "\n" });
            }
            out.push_str(&" ".repeat(level));
            out.push('}');
        }
        J::String(s) => out.push_str(&text(s)),
        J::Number(n) if n.is_f64() => {
            let f = n.as_f64().unwrap_or(f64::NAN);
            out.push_str(&duckosm_build::place::py_float(f));
        }
        other => out.push_str(&other.to_string()),
    }
}

/// A JSON text as Python writes it (ensure_ascii=True).
fn text(s: &str) -> String {
    let mut e = String::new();
    for c in serde_json::to_string(s).expect("JSON of a text").chars() {
        if c.is_ascii() {
            e.push(c);
        } else {
            let mut b = [0u16; 2];
            for u in c.encode_utf16(&mut b) {
                e += &format!("\\u{u:04x}");
            }
        }
    }
    e
}

fn ts(micros: i64) -> String {
    // Python's str(datetime): "YYYY-MM-DD HH:MM:SS[.ffffff]"
    let (secs, us) = (micros.div_euclid(1_000_000), micros.rem_euclid(1_000_000));
    let (days, sod) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // civil from days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    let base = format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}", sod / 3600, sod % 3600 / 60, sod % 60);
    if us == 0 { base } else { format!("{base}.{us:06}") }
}

/// A DuckDB value as JSON, as Python gets it from DuckDB (a FLOAT is widened to a double) and `json.dumps(...,
/// default=str)` writes it (a time as its text).
pub fn value(v: Value) -> J {
    match v {
        Value::Null => J::Null,
        Value::Boolean(b) => json!(b),
        Value::TinyInt(i) => json!(i),
        Value::SmallInt(i) => json!(i),
        Value::Int(i) => json!(i),
        Value::BigInt(i) => json!(i),
        Value::HugeInt(i) => json!(i as f64),
        Value::UTinyInt(i) => json!(i),
        Value::USmallInt(i) => json!(i),
        Value::UInt(i) => json!(i),
        Value::UBigInt(i) => json!(i),
        Value::Float(f) => float(f as f64),
        Value::Double(f) => float(f),
        Value::Text(s) => json!(s),
        Value::Timestamp(unit, t) => {
            let us = match unit {
                TimeUnit::Second => t * 1_000_000,
                TimeUnit::Millisecond => t * 1000,
                TimeUnit::Microsecond => t,
                TimeUnit::Nanosecond => t / 1000,
            };
            json!(ts(us))
        }
        Value::List(l) | Value::Array(l) => J::Array(l.into_iter().map(value).collect()),
        Value::Map(m) => J::Object(m.iter().map(|(k, v)| (key(k.clone()), value(v.clone()))).collect()),
        Value::Struct(s) => J::Object(s.iter().map(|(k, v)| (k.clone(), value(v.clone()))).collect()),
        other => json!(format!("{other:?}")),
    }
}

fn key(v: Value) -> String {
    match value(v) {
        J::String(s) => s,
        o => o.to_string(),
    }
}

/// A float (NaN and infinity have no JSON number: as text).
fn float(f: f64) -> J {
    serde_json::Number::from_f64(f).map_or_else(|| J::String(format!("{f}")), J::Number)
}
