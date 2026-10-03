use super::*;

#[test]
fn test_redact() {
    let mut owned = RString::new();
    owned.push_rstr("this is a password: ");
    owned.push_redacted("hunter2");
    owned.push_rstr("... now the secret is out!");

    assert_eq!(
        format!("See {owned}"),
        "See this is a password: ***... now the secret is out!"
    );

    assert_eq!(
        owned.to_exposed(),
        "this is a password: hunter2... now the secret is out!"
    );
}

#[test]
fn test_eq() {
    let mut a = RString::new();
    let mut b = RString::new();

    a.push_rstr("prefix");
    assert!(a.push_redacted("foo"));
    assert!(a.push_redacted("bar"));
    a.push_rstr("suffix");

    b.push_rstr("prefix");
    assert!(b.push_redacted("fo"));
    assert!(b.push_redacted("obar"));
    b.push_rstr("suffix");

    assert_eq!(a, b);
    assert_eq!(b, a);
    assert!(a.exposed_eq(&b));
    assert!(b.exposed_eq(&a));

    assert!(a.str_eq("prefixfoobarsuffix"));
    assert!(b.str_eq("prefixfoobarsuffix"));
}

#[test]
fn test_push_redacted_invalid_leaves_string_unmodified() {
    let mut s = RString::new();
    s.push_rstr("prefix");

    assert!(!s.push_redacted("ok\u{7}"));
    assert!(!s.push_redacted("snowman \u{2603}"));
    assert!(!s.push_redacted("tab\there"));

    assert_eq!(s.as_raw(), "prefix");
    assert_eq!(s.to_exposed(), "prefix");
    assert!(RString::redacted("bad\n").is_none());
    assert!(RString::redacted("good").is_some());
}

#[test]
fn test_redacted_roundtrip_printable_ascii() {
    let all = (0x20u8..0x7f).map(char::from).collect::<String>();
    let s = RString::redacted(&all).unwrap();
    assert_eq!(s.to_exposed(), all);
    assert_eq!(s.to_string(), "***");
}

#[test]
fn test_arbitrary_tag_input_decodes_safely() {
    // A tag start marker followed by ordinary text was not produced by
    // `push_redacted`, so the redacted span decodes to replacement chars
    // instead of underflowing.
    let s = RStr::new("a\u{E0001}bc");
    assert_eq!(s.to_exposed(), "a\u{FFFD}\u{FFFD}");
    assert_eq!(s.to_string(), "a***");

    // Same with an explicit end marker and trailing public text.
    let s = RStr::new("a\u{E0001}x\u{E007F}y");
    assert_eq!(s.to_exposed(), "a\u{FFFD}y");

    // Tag-range characters outside of printable ASCII and unpaired markers.
    let s = RString::from(String::from(
        "\u{E0001}\u{E0000}\u{E0005}\u{E0041}\u{E007F}",
    ));
    assert_eq!(s.to_exposed(), "\u{FFFD}\u{FFFD}A");

    let s = RStr::new("\u{E0001}");
    assert_eq!(s.to_exposed(), "");
    let s = RStr::new("\u{E0001}\u{E0001}");
    assert_eq!(s.to_exposed(), "\u{FFFD}");

    assert!(s.find('x').is_none());
    assert!(!RStr::new("\u{E0001}abc").str_eq("abc"));
}

#[test]
fn test_find_returns_raw_offsets() {
    let mut s = RString::from(String::from("ab "));
    assert!(s.push_redacted("sec/ret"));
    s.push_rstr("cd=e");

    let raw = s.as_raw();

    // Match after a redacted span slices the raw string.
    let at = s.find('=').unwrap();
    assert_eq!(&raw[at..], "=e");
    let at = s.find('d').unwrap();
    assert_eq!(&raw[at..], "d=e");

    // Public match before the span.
    assert_eq!(s.find('b'), Some(1));

    // Redacted characters match by decoded value and point at the encoded char.
    let at = s.find('/').unwrap();
    assert_eq!(decode(raw[at..].chars().next().unwrap()), '/');
    assert_eq!(at, "ab ".len() + TAG_START.len() + 3 * 4);

    // Undecodable characters count their real length.
    let s = RStr::new("\u{E0001}\u{E0000}\u{E0041}\u{E007F}=");
    assert_eq!(s.find('A'), Some(TAG_START.len() + 4));
    assert_eq!(&s.as_raw()[s.find('=').unwrap()..], "=");
}
