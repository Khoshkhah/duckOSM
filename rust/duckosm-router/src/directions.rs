//! duckOSM `directions.py`: turn-by-turn steps for a route (route-guidance's manoeuvre rules, the OSRM model). The same
//! rules, with the same math functions, so the steps and their texts equal duckOSM's.

use crate::geom::Pt;

const COMPASS: [&str; 8] = ["north", "northeast", "east", "southeast", "south", "southwest", "west", "northwest"];
const R: f64 = 6371000.0;

/// A road of the route: its name, highway, length (or the metres used), the node it starts at, whether it is a roundabout,
/// its line, and the lines of all roads that leave its start node.
#[derive(Clone, Debug)]
pub struct Road {
    pub mode: String,
    pub name: Option<String>,
    pub highway: Option<String>,
    pub length_m: f64,
    pub roundabout: bool,
    pub line: Vec<Pt>,
    pub out: Vec<Vec<Pt>>,
}

/// One step: type (depart, turn, fork, new name, roundabout, mode, arrive), modifier, name, mode, where, metres, the exit
/// of a roundabout, and the text.
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub kind: String,
    pub modifier: String,
    pub name: Option<String>,
    pub mode: String,
    pub at: Pt,
    pub length_m: f64,
    pub exit: Option<u32>,
    pub text: String,
}

/// Python's `x % y` for floats with y > 0.
fn pmod(x: f64, y: f64) -> f64 {
    let r = x % y;
    if r < 0.0 { r + y } else if r == 0.0 { 0.0 } else { r }
}

fn hav(a: Pt, b: Pt) -> f64 {
    let (dl, dn) = ((b.1 - a.1).to_radians(), (b.0 - a.0).to_radians());
    let h = (dl / 2.0).sin().powi(2) + a.1.to_radians().cos() * b.1.to_radians().cos() * (dn / 2.0).sin().powi(2);
    2.0 * R * h.sqrt().asin()
}

fn brg(a: Pt, b: Pt) -> f64 {
    let (f1, f2, dn) = (a.1.to_radians(), b.1.to_radians(), (b.0 - a.0).to_radians());
    let y = dn.sin() * f2.cos();
    let x = f1.cos() * f2.sin() - f1.sin() * f2.cos() * dn.cos();
    pmod(y.atan2(x).to_degrees() + 360.0, 360.0)
}

/// The heading over the first / last 18 m of a line.
fn head(c: &[Pt], from_start: bool) -> f64 {
    let mut acc = 0.0;
    if from_start {
        for i in 1..c.len() {
            acc += hav(c[i - 1], c[i]);
            if acc >= 18.0 || i == c.len() - 1 {
                return brg(c[0], c[i]);
            }
        }
    } else if c.len() >= 2 {
        for i in (0..c.len() - 1).rev() {
            acc += hav(c[i], c[i + 1]);
            if acc >= 18.0 || i == 0 {
                return brg(c[i], c[c.len() - 1]);
            }
        }
    }
    0.0
}

/// + is right.
fn turn(b0: f64, b1: f64) -> f64 {
    pmod(b1 - b0 + 540.0, 360.0) - 180.0
}

fn modifier(a: f64) -> String {
    let (x, side) = (a.abs(), if a > 0.0 { "right" } else { "left" });
    if x < 20.0 {
        "straight".into()
    } else if x >= 170.0 {
        "uturn".into()
    } else if x < 45.0 {
        format!("slight {side}")
    } else if x < 135.0 {
        side.into()
    } else {
        format!("sharp {side}")
    }
}

fn is_turn(m: &str) -> bool {
    matches!(m, "left" | "right" | "uturn") || m.starts_with("sharp")
}

fn nth(n: u32) -> String {
    let suffix = if (11..=13).contains(&(n % 100)) {
        "th"
    } else {
        match n % 10 {
            1 => "st",
            2 => "nd",
            3 => "rd",
            _ => "th",
        }
    };
    format!("{n}{suffix}")
}

fn verb(mode: &str) -> &'static str {
    match mode {
        "walking" => "on foot",
        "cycling" => "by bike",
        "driving" => "by car",
        _ => "",
    }
}

fn say(s: &Step) -> String {
    let (onto, on) = match s.name.as_deref().filter(|n| !n.is_empty()) {
        Some(n) => (format!(" onto {n}"), format!(" on {n}")),
        None => (String::new(), String::new()),
    };
    match s.kind.as_str() {
        "depart" => format!("{}{}{on}", if s.mode == "walking" { "Walk " } else { "Head " }, s.modifier),
        "turn" if s.modifier == "uturn" => format!("Make a U-turn{onto}"),
        "turn" => format!("Turn {}{onto}", s.modifier),
        "fork" => format!("Keep {} at the fork{onto}", s.modifier),
        "new name" => format!("Continue{onto}"),
        "roundabout" => format!("At the roundabout, take the {} exit{onto}", nth(s.exit.unwrap_or(0))),
        "mode" => format!("Continue {}{onto}", verb(&s.mode)),
        _ => "Arrive at your destination".into(),
    }
}

fn step(kind: &str, modifier: &str, e: &Road) -> Step {
    Step { kind: kind.into(), modifier: modifier.into(), name: e.name.clone(), mode: e.mode.clone(), at: e.line[0], length_m: 0.0, exit: None, text: String::new() }
}

/// The steps of a route whose roads, in order, are `roads` (duckOSM `directions`). Empty for no roads.
pub fn directions(roads: &[Road]) -> Vec<Step> {
    if roads.is_empty() {
        return Vec::new();
    }
    let truthy = |n: &Option<String>| n.as_deref().is_some_and(|x| !x.is_empty());
    let same = |a: &Road, b: &Road| if truthy(&a.name) || truthy(&b.name) { a.name == b.name } else { a.highway == b.highway };
    let mut steps: Vec<Step> = Vec::new();
    let (mut cur, mut rb): (Option<Step>, Option<Step>) = (None, None);
    let mut out_b = 0.0;
    for (i, e) in roads.iter().enumerate() {
        let (e_in, e_out) = (head(&e.line, true), head(&e.line, false));
        if e.roundabout {
            if rb.is_none() {
                if let Some(c) = cur.take() {
                    steps.push(c);
                }
                let mut r = step("roundabout", "", e);
                r.name = None;
                r.exit = Some(0);
                rb = Some(r);
            }
            let r = rb.as_mut().expect("set above");
            r.exit = Some(r.exit.unwrap_or(0) + 1);
            r.length_m += e.length_m;
            out_b = e_out;
            continue;
        }
        if let Some(mut r) = rb.take() {
            r.name = e.name.clone();
            r.length_m += e.length_m;
            cur = Some(r);
            out_b = e_out;
            continue;
        }
        match &cur {
            None => {
                let ix = (pmod(e_in + 22.5, 360.0) / 45.0).floor() as usize;
                cur = Some(step("depart", COMPASS[ix.min(7)], e));
            }
            Some(_) => {
                let prev = &roads[i - 1];
                let mine = turn(out_b, e_in);
                let m = modifier(mine);
                let sharp = m == "uturn" || m.starts_with("sharp");
                let junction = e.out.len() >= 3;
                let named = !same(prev, e) && truthy(&e.name);
                let mut fork: Option<&str> = None;
                if junction && !is_turn(&m) {
                    let sibs: Vec<f64> = e.out.iter().map(|c| head(c, true)).filter(|&b| turn(e_in, b).abs() > 8.0).map(|b| turn(out_b, b)).collect();
                    if (12.0..50.0).contains(&mine.abs())
                        && !sibs.iter().any(|t| t.abs() < 12.0)
                        && sibs.iter().any(|&t| t.abs() < 50.0 && (t < 0.0) != (mine < 0.0))
                    {
                        fork = Some(if mine < 0.0 { "left" } else { "right" });
                    }
                }
                let nxt = if e.mode != prev.mode {
                    Some(step("mode", &m, e))
                } else if sharp || (junction && is_turn(&m)) {
                    Some(step("turn", &m, e))
                } else if let Some(f) = fork {
                    Some(step("fork", f, e))
                } else if named {
                    Some(step("new name", &m, e))
                } else {
                    None
                };
                if let Some(n) = nxt {
                    steps.push(cur.take().expect("some"));
                    cur = Some(n);
                }
            }
        }
        cur.as_mut().expect("set").length_m += e.length_m;
        out_b = e_out;
    }
    match rb {
        Some(r) => steps.push(r),
        None => steps.extend(cur),
    }
    let last = roads.last().expect("not empty");
    steps.push(Step {
        kind: "arrive".into(),
        modifier: String::new(),
        name: None,
        mode: last.mode.clone(),
        at: *last.line.last().expect("a line"),
        length_m: 0.0,
        exit: None,
        text: String::new(),
    });
    for s in &mut steps {
        s.text = say(s);
    }
    steps
}
