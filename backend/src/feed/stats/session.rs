//! Session-key derivation + feed-value parsing for stats collection.
//! Ported from the standalone `stats` model crate (`crates/model/src/lib.rs`).

use chrono::{DateTime, Utc};

/// Derive a compact, stable 64-bit session id from `(cid, logon_time)`.
///
/// VATSIM enforces a single login per cid, so this pair is unique per connection; a reconnect gets a
/// new `logon_time` and therefore a new id. FNV-1a is used (not `DefaultHasher`) so the value is
/// stable across builds.
pub fn session_id(cid: i32, logon_time: &str) -> i64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;
    for &b in cid.to_le_bytes().iter() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(PRIME);
    }
    hash ^= b':' as u64;
    hash = hash.wrapping_mul(PRIME);
    for &b in logon_time.as_bytes() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(PRIME);
    }
    hash as i64
}

/// Parse a feed RFC3339 timestamp (with up to 7 fractional digits + `Z`).
pub fn parse_ts(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

/// Parse a filed altitude string ("28000", "FL340", "F340") into feet.
pub fn parse_filed_altitude(s: &str) -> Option<i32> {
    let t = s.trim().to_uppercase();
    if t.is_empty() {
        return None;
    }
    if let Some(rest) = t.strip_prefix("FL").or_else(|| t.strip_prefix('F')) {
        rest.trim().parse::<i32>().ok().map(|fl| fl * 100)
    } else {
        t.parse::<i32>().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_id_is_stable_and_distinct() {
        let a = session_id(1816474, "2026-08-09T04:28:17.9210242Z");
        let b = session_id(1816474, "2026-08-09T04:28:17.9210242Z");
        let c = session_id(1816474, "2026-08-09T05:00:00.0000000Z");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn filed_altitude_parses() {
        assert_eq!(parse_filed_altitude("28000"), Some(28000));
        assert_eq!(parse_filed_altitude("FL340"), Some(34000));
        assert_eq!(parse_filed_altitude("F340"), Some(34000));
        assert_eq!(parse_filed_altitude("garbage"), None);
        assert_eq!(parse_filed_altitude(""), None);
    }
}
