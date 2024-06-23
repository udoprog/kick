use super::*;

#[test]
fn test_redact() {
    let mut owned = RString::new();
    owned.push_str("this is a password: ");
    owned.push_redacted("hunter2");
    owned.push_str("... now the secret is out!");

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

    a.push_str("prefix");
    assert!(a.push_redacted("foo"));
    assert!(a.push_redacted("bar"));
    a.push_str("suffix");

    b.push_str("prefix");
    assert!(b.push_redacted("fo"));
    assert!(b.push_redacted("obar"));
    b.push_str("suffix");

    assert_eq!(a, b);
    assert_eq!(b, a);
    assert!(a.exposed_eq(&b));
    assert!(b.exposed_eq(&a));

    assert!(a.str_eq("prefixfoobarsuffix"));
    assert!(b.str_eq("prefixfoobarsuffix"));
}
