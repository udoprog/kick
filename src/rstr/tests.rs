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
