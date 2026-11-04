//! The duckOSM `edge_id` (v2). It is the DuckDB `hash()` of BIGINT and VARCHAR arguments, as of
//! DuckDB 1.3 and later. The Python reference is `tools/edge_id_reference.py`.

const C: u64 = 0xd6e8_feb8_6659_fd93; // multiplier of the integer mix
const S: u64 = 0xc6a4_a793_5bd1_e995; // multiplier of the string length
const SEED: u64 = 0xe17a_1465; // start value of the string hash

/// Hash of one 64-bit integer. A negative number is read as unsigned (two's complement).
pub fn mix(x: u64) -> u64 {
    let mut x = x;
    x ^= x >> 32;
    x = x.wrapping_mul(C);
    x ^= x >> 32;
    x = x.wrapping_mul(C);
    x ^= x >> 32;
    x
}

fn combine(h: u64, other: u64) -> u64 {
    (h ^ (h >> 32)).wrapping_mul(C) ^ other
}

/// Hash of a text value.
pub fn hash_str(s: &str) -> u64 {
    let b = s.as_bytes();
    let mut h = SEED ^ (b.len() as u64).wrapping_mul(S);
    for chunk in b.chunks(8) {
        let mut w = [0u8; 8];
        w[..chunk.len()].copy_from_slice(chunk);
        h = (h ^ u64::from_le_bytes(w)).wrapping_mul(C);
    }
    mix(h)
}

/// `hash(osm_id, source, target)`.
pub fn hash3(osm_id: i64, source: i64, target: i64) -> u64 {
    combine(
        combine(mix(osm_id as u64), mix(source as u64)),
        mix(target as u64),
    )
}

/// `edge_id = (hash(osm_id, source, target) >> 1)::BIGINT`.
pub fn edge_id(osm_id: i64, source: i64, target: i64) -> i64 {
    (hash3(osm_id, source, target) >> 1) as i64
}

/// `hash(a, b, text)` for two BIGINT and a VARCHAR.
pub fn hash_int_int_str(a: i64, b: i64, s: &str) -> u64 {
    combine(combine(mix(a as u64), mix(b as u64)), hash_str(s))
}

/// DuckDB writes a list as `[1, 2, 3]`. The virtual node ids hash this text.
pub fn refs_text(refs: &[i64]) -> String {
    let mut s = String::with_capacity(2 + refs.len() * 12);
    s.push('[');
    for (i, r) in refs.iter().enumerate() {
        if i > 0 {
            s.push_str(", ");
        }
        s.push_str(&r.to_string());
    }
    s.push(']');
    s
}

/// Virtual node of a split self-loop: `-(hash(osm_id, source, refs::VARCHAR) >> 2)`.
pub fn virtual_node_loop(osm_id: i64, source: i64, refs: &[i64]) -> i64 {
    let h = combine(
        combine(mix(osm_id as u64), mix(source as u64)),
        hash_str(&refs_text(refs)),
    );
    -((h >> 2) as i64)
}

/// Virtual node of a split arc: `-(hash(osm_id, source, target, refs::VARCHAR) >> 2)`.
pub fn virtual_node_arc(osm_id: i64, source: i64, target: i64, refs: &[i64]) -> i64 {
    let h = combine(
        combine(
            combine(mix(osm_id as u64), mix(source as u64)),
            mix(target as u64),
        ),
        hash_str(&refs_text(refs)),
    );
    -((h >> 2) as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Values from DuckDB 1.5.3 (and 1.3.2, 1.4.4):
    //   SELECT hash(1::BIGINT, 2::BIGINT, 3::BIGINT)               -> 12722334483198565868
    //   SELECT hash(123456789::BIGINT, 987654321::BIGINT, -555::BIGINT) -> 11030431678157071902
    //   SELECT hash('[1, 2, 3]')                                     -> 12537961803640074991
    //   SELECT hash(7::BIGINT, 8::BIGINT, '[1, 2, 3]')               -> 2369374505249423874
    //   SELECT hash('')                                              -> mix(0xe17a1465)
    #[test]
    fn integer_hash() {
        assert_eq!(hash3(1, 2, 3), 12722334483198565868);
        assert_eq!(hash3(123456789, 987654321, -555), 11030431678157071902);
    }

    #[test]
    fn text_hash() {
        assert_eq!(hash_str("[1, 2, 3]"), 12537961803640074991);
        assert_eq!(hash_str(""), mix(0xe17a_1465));
        let h = combine(combine(mix(7), mix(8)), hash_str("[1, 2, 3]"));
        assert_eq!(h, 2369374505249423874);
    }

    #[test]
    fn edge_id_is_signed_and_positive() {
        assert_eq!(edge_id(1, 2, 3), (12722334483198565868u64 >> 1) as i64);
        assert!(edge_id(1, 2, 3) > 0);
    }
}
