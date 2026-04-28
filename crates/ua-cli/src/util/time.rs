//! Tiny epoch → ISO-8601 helper, deduplicated out of `analyze.rs` and
//! `knowledge.rs`. Avoids pulling in `chrono` for a one-liner format.

/// Current wall-clock time as `YYYY-MM-DDTHH:MM:SSZ`. Falls back to
/// the unix epoch on clock skew (the same fallback both call sites
/// used before the dedup).
pub fn iso8601_now() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let (y, mo, d, h, mi, s) = epoch_to_ymdhms(now as i64);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// Howard Hinnant's `days_from_civil` reversed — convert a unix
/// timestamp in *seconds* to `(year, month, day, hour, minute, second)`
/// in UTC. Lifted verbatim from the original duplicated copy so the
/// switch is a true no-op.
pub fn epoch_to_ymdhms(secs: i64) -> (i64, u32, u32, u32, u32, u32) {
    let z = secs.div_euclid(86_400) + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let mo = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let y = if mo <= 2 { y + 1 } else { y };
    let secs_of_day = secs.rem_euclid(86_400);
    let h = (secs_of_day / 3600) as u32;
    let mi = ((secs_of_day % 3600) / 60) as u32;
    let s = (secs_of_day % 60) as u32;
    (y, mo, d, h, mi, s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_zero_is_unix_origin() {
        assert_eq!(epoch_to_ymdhms(0), (1970, 1, 1, 0, 0, 0));
    }

    #[test]
    fn known_timestamp_round_trips() {
        // 2024-01-02T03:04:05Z
        let secs = 1_704_164_645_i64;
        assert_eq!(epoch_to_ymdhms(secs), (2024, 1, 2, 3, 4, 5));
    }

    #[test]
    fn iso_format_is_well_formed() {
        let s = iso8601_now();
        assert_eq!(s.len(), 20);
        assert!(s.ends_with('Z'));
        assert_eq!(s.as_bytes()[4], b'-');
        assert_eq!(s.as_bytes()[7], b'-');
        assert_eq!(s.as_bytes()[10], b'T');
        assert_eq!(s.as_bytes()[13], b':');
        assert_eq!(s.as_bytes()[16], b':');
    }
}
