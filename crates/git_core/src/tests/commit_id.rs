use git2::Oid;

use std::collections::{BTreeSet, HashSet};

use crate::{CommitId, ParseCommitIdError};

const HEX: &str = "0123456789abcdef0123456789abcdef01234567";

fn id(hex: &str) -> CommitId {
    hex.parse().unwrap()
}

// ------------------------------------------------------------------ display / debug

#[test]
fn displays_as_40_lowercase_hex_characters() {
    let shown = id(HEX).to_string();
    assert_eq!(shown, HEX);
    assert_eq!(shown.len(), 40);
}

#[test]
fn display_honours_width_and_padding() {
    assert_eq!(format!("{:>45}", id(HEX)), format!("     {HEX}"));
}

#[test]
fn debug_shows_the_hex() {
    assert_eq!(format!("{:?}", id(HEX)), format!("CommitId({HEX})"));
}

#[test]
fn short_is_the_first_seven_characters() {
    assert_eq!(id(HEX).short(), "0123456");
    assert!(HEX.starts_with(&id(HEX).short()));
}

// ---------------------------------------------------------------------- parsing

#[test]
fn parse_and_display_round_trip() {
    assert_eq!(id(HEX).to_string().parse::<CommitId>().unwrap(), id(HEX));
}

#[test]
fn uppercase_hex_is_accepted_and_normalised() {
    let upper = id(&HEX.to_uppercase());
    assert_eq!(upper, id(HEX));
    assert_eq!(upper.to_string(), HEX, "always shown in lowercase");
}

#[test]
fn all_zero_id_is_valid() {
    assert_eq!(id(&"0".repeat(40)).to_string(), "0".repeat(40));
}

#[test]
fn invalid_ids_are_rejected() {
    let invalid = [
        "",
        "0123456",                    // abbreviated
        &HEX[..39],                   // one short
        &format!("{HEX}0"),           // one long
        &format!("{}g", &HEX[..39]),  // not hex
        &format!(" {}", &HEX[..39]),  // leading space
        &format!("{HEX}\n"),          // trailing newline
        &format!("0x{}", &HEX[..38]), // prefix
        &"é".repeat(20),              // 40 bytes, not ascii
    ];
    for text in invalid {
        let err = text.parse::<CommitId>().unwrap_err();
        assert_eq!(err, ParseCommitIdError(text.to_owned()), "{text:?}");
    }
}

#[test]
fn parse_error_names_the_input() {
    let err = "nope".parse::<CommitId>().unwrap_err();
    assert!(err.to_string().contains("`nope`"), "{err}");
}

// ------------------------------------------------------------------ conversions

#[test]
fn from_oid_keeps_the_same_hex() {
    let oid = Oid::from_str(HEX).unwrap();
    assert_eq!(CommitId::from_oid(oid).to_string(), oid.to_string());
}

// ------------------------------------------------------------ equality / ordering

#[test]
fn can_be_used_as_a_hash_and_ordered_key() {
    let (a, b) = (id(&"a".repeat(40)), id(&"b".repeat(40)));
    let copy = a; // Copy: `a` is still usable
    assert_eq!(a, copy);
    assert_ne!(a, b);

    assert_eq!(HashSet::from([a, b, copy]).len(), 2);
    assert_eq!(
        BTreeSet::from([b, a]).into_iter().collect::<Vec<_>>(),
        [a, b]
    );
}

#[test]
fn ordering_matches_the_hex_text() {
    let low = id(&format!("00{}", "f".repeat(38)));
    let high = id(&format!("01{}", "0".repeat(38)));
    assert!(low < high);
    assert!(low.to_string() < high.to_string());
}

// ------------------------------------------------------------------------- serde

#[test]
fn serializes_as_a_json_string() {
    assert_eq!(
        serde_json::to_string(&id(HEX)).unwrap(),
        format!("\"{HEX}\"")
    );
}

#[test]
fn json_round_trip() {
    let json = serde_json::to_string(&id(HEX)).unwrap();
    assert_eq!(serde_json::from_str::<CommitId>(&json).unwrap(), id(HEX));
}

#[test]
fn works_as_a_struct_field() {
    #[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
    struct Response {
        id: CommitId,
        ids: Vec<CommitId>,
    }
    let response = Response {
        id: id(HEX),
        ids: vec![id(&"a".repeat(40)), id(&"b".repeat(40))],
    };

    let json = serde_json::to_string(&response).unwrap();
    assert!(json.contains(&format!("\"id\":\"{HEX}\"")), "{json}");
    assert_eq!(serde_json::from_str::<Response>(&json).unwrap(), response);
}

#[test]
fn deserializing_garbage_is_an_error() {
    for json in [
        "\"abc\"",
        "\"\"",
        "123",
        "null",
        "[]",
        &format!("\"{HEX}0\""),
    ] {
        assert!(serde_json::from_str::<CommitId>(json).is_err(), "{json}");
    }
}

#[test]
fn deserialize_error_explains_what_is_wrong() {
    let err = serde_json::from_str::<CommitId>("\"abc\"").unwrap_err();
    assert!(err.to_string().contains("40 hexadecimal"), "{err}");
}
