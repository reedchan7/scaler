use scaler::detach::id::RunId;

#[test]
fn run_id_format_is_timestamp_plus_4_hex() {
    let id = RunId::generate();
    let s = id.as_str();
    assert_eq!(s.len(), 20, "id = {s:?}");
    let bytes = s.as_bytes();
    assert_eq!(bytes[8], b'-');
    assert_eq!(bytes[15], b'-');
    for &b in &bytes[..8] {
        assert!(b.is_ascii_digit(), "date byte {b} in {s:?}");
    }
    for &b in &bytes[9..15] {
        assert!(b.is_ascii_digit(), "time byte {b} in {s:?}");
    }
    for &b in &bytes[16..20] {
        assert!(
            b.is_ascii_hexdigit() && !b.is_ascii_uppercase(),
            "hex byte {b} in {s:?}"
        );
    }
}

#[test]
fn run_id_parse_accepts_canonical_form() {
    let id = RunId::parse("20260408-143022-a1b2").expect("valid id");
    assert_eq!(id.as_str(), "20260408-143022-a1b2");
}

#[test]
fn run_id_parse_rejects_malformed_input() {
    assert!(RunId::parse("").is_none());
    assert!(RunId::parse("20260408-143022").is_none());
    assert!(RunId::parse("20260408_143022-a1b2").is_none());
    assert!(RunId::parse("20260408-143022-A1B2").is_none());
    assert!(RunId::parse("20260408-143022-a1b2z").is_none());
}

#[test]
fn run_id_two_generated_ids_differ() {
    let a = RunId::generate();
    let b = RunId::generate();
    assert_ne!(a.as_str(), b.as_str(), "entropy source produced duplicate");
}

#[test]
fn run_id_prefix_match_exact_wins() {
    let all = vec![
        RunId::parse("20260408-143022-a1b2").unwrap(),
        RunId::parse("20260408-143022-a1b3").unwrap(),
    ];
    let hit = RunId::find_by_prefix("20260408-143022-a1b2", &all).unwrap();
    assert_eq!(hit.as_str(), "20260408-143022-a1b2");
}

#[test]
fn run_id_prefix_match_unique_prefix_wins() {
    let all = vec![
        RunId::parse("20260408-143022-a1b2").unwrap(),
        RunId::parse("20260408-090000-cdef").unwrap(),
    ];
    let hit = RunId::find_by_prefix("20260408-143", &all).unwrap();
    assert_eq!(hit.as_str(), "20260408-143022-a1b2");
}

#[test]
fn run_id_prefix_match_ambiguous_returns_none() {
    let all = vec![
        RunId::parse("20260408-143022-a1b2").unwrap(),
        RunId::parse("20260408-143022-a1b3").unwrap(),
    ];
    assert!(RunId::find_by_prefix("20260408-143022-a1b", &all).is_none());
}
