use super::*;

#[test]
fn matrix_test() {
    let mut tree = Tree::new();
    tree.insert(["matrix", "a"], "1");
    tree.insert(["matrix", "b"], "2");

    let eval = Eval::new(&tree);

    assert_eq!(eval.test("matrix.a == '1'"), Ok(true));
    assert_eq!(eval.test("matrix.a != '2'"), Ok(true));
    assert_eq!(eval.test("matrix.a != matrix.b"), Ok(true));
    assert_eq!(eval.test("matrix.a == matrix.b"), Ok(false));
    assert_eq!(
        eval.test("matrix.a == matrix.b || matrix.a != matrix.b"),
        Ok(true)
    );
}

#[test]
fn or_test() {
    let mut tree = Tree::new();
    tree.insert(["matrix", "bar"], "right");
    let eval = Eval::new(&tree);

    assert_eq!(
        eval.expr("matrix.foo || matrix.bar"),
        Ok(Expr::String(Cow::Borrowed(RStr::new("right"))))
    );
}

#[test]
fn and_test() {
    let mut tree = Tree::new();
    tree.insert(["matrix", "foo"], "wrong");
    tree.insert(["matrix", "bar"], "right");

    let eval = Eval::new(&tree);

    assert_eq!(
        eval.expr("matrix.foo && matrix.bar"),
        Ok(Expr::String(Cow::Borrowed(RStr::new("right"))))
    );

    assert_eq!(eval.expr("matrix.baz && matrix.bar"), Ok(Expr::Null));
}

#[test]
fn group() {
    let mut tree = Tree::new();
    tree.insert(["matrix", "foo"], "wrong");
    tree.insert(["matrix", "bar"], "right");

    let eval = Eval::new(&tree);

    assert_eq!(
        eval.expr("${{ matrix.foo }} && matrix.bar"),
        Ok(Expr::String(Cow::Borrowed(RStr::new("right"))))
    );

    assert_eq!(eval.expr("matrix.baz && matrix.bar"), Ok(Expr::Null));
}

#[test]
fn not() {
    let mut tree = Tree::new();
    tree.insert(["matrix", "foo"], "");
    tree.insert(["matrix", "bar"], "right");

    let eval = Eval::new(&tree);

    assert_eq!(eval.expr("!matrix.foo"), Ok(Expr::Bool(true)));
    assert_eq!(eval.expr("!matrix.bar"), Ok(Expr::Bool(false)));
    assert_eq!(eval.expr("!matrix.baz"), Ok(Expr::Bool(true)));
}

#[test]
fn lazy_expansion() {
    let mut tree = Tree::new();
    tree.insert(["matrix", "ref"], "refs/heads/main");
    tree.insert(["matrix", "ref2"], "refs/heads/feature");
    let eval = Eval::new(&tree);

    assert_eq!(
        eval.expr(
            r#"
        matrix.ref == 'refs/heads/main' && 'value_for_main_branch' ||
        'value_for_other_branches'
        "#
        ),
        Ok(Expr::String(Cow::Borrowed(RStr::new(
            "value_for_main_branch"
        ))))
    );

    assert_eq!(
        eval.expr(
            r#"
        matrix.ref2 == 'refs/heads/main' && 'value_for_main_branch' ||
        'value_for_other_branches'
        "#
        ),
        Ok(Expr::String(Cow::Borrowed(RStr::new(
            "value_for_other_branches"
        ))))
    );
}

#[test]
fn comparisons() {
    let eval = Eval::empty();
    assert_eq!(eval.expr("100"), Ok(Expr::Float(100.0)));
    assert!(eval.expr("nan").unwrap().as_f64().is_nan());
    assert!(eval.expr("'foo'").unwrap().as_f64().is_nan());
    assert!(eval.expr("null").unwrap().as_f64() == 0.0);
    assert_eq!(eval.expr("100 <= 100"), Ok(Expr::Bool(true)));
    assert_eq!(eval.expr("100 < 100"), Ok(Expr::Bool(false)));
    assert_eq!(eval.expr("100 >= 100"), Ok(Expr::Bool(true)));
    assert_eq!(eval.expr("100 > 100"), Ok(Expr::Bool(false)));
    // null is treated as 0
    assert_eq!(eval.expr("null > 100"), Ok(Expr::Bool(false)));
    assert_eq!(eval.expr("null < 100"), Ok(Expr::Bool(true)));
    assert_eq!(eval.expr("null == 0"), Ok(Expr::Bool(true)));
    assert_eq!(eval.expr("null == false"), Ok(Expr::Bool(true)));
    // nan can't be compared
    assert_eq!(eval.expr("nan > 100"), Ok(Expr::Bool(false)));
    assert_eq!(eval.expr("nan < 100"), Ok(Expr::Bool(false)));
    assert_eq!(eval.expr("nan == 0"), Ok(Expr::Bool(false)));
    assert_eq!(eval.expr("nan == false"), Ok(Expr::Bool(false)));
    assert_eq!(eval.expr("!nan"), Ok(Expr::Bool(true)));
    assert_eq!(eval.expr("!!nan"), Ok(Expr::Bool(false)));
    assert_eq!(eval.expr("nan == nan"), Ok(Expr::Bool(false)));
    // non-zero strings are nan
    assert_eq!(eval.expr("'foo' > 100"), Ok(Expr::Bool(false)));
    assert_eq!(eval.expr("'foo' < 100"), Ok(Expr::Bool(false)));
    assert_eq!(eval.expr("'foo' == 0"), Ok(Expr::Bool(false)));
    assert_eq!(eval.expr("'foo' == false"), Ok(Expr::Bool(false)));
    assert_eq!(eval.expr("!!'foo'"), Ok(Expr::Bool(true)));
}

#[test]
fn lookup_star() {
    let mut tree = Tree::new();
    tree.insert(["matrix", "a"], "first");
    tree.insert(["matrix", "b"], "second");
    let eval = Eval::new(&tree);

    let expected = Expr::Array(["first".into(), "second".into()].into());
    assert_eq!(eval.expr("matrix.*"), Ok(expected));
}

#[test]
fn function() {
    let mut tree = Tree::new();
    tree.insert(["matrix", "a"], "true");
    tree.insert(["matrix", "b"], "false");
    tree.insert(["matrix", "c"], "[1, 2, 3, 4]");
    let eval = Eval::new(&tree);
    assert_eq!(eval.expr("fromJSON(matrix.a)"), Ok(Expr::Bool(true)));
    assert_eq!(eval.expr("fromJSON(matrix.b)"), Ok(Expr::Bool(false)));
    assert_eq!(
        eval.expr("fromJSON(matrix.b) || true"),
        Ok(Expr::Bool(true))
    );

    assert_eq!(
        eval.expr("fromJSON(matrix.c)"),
        Ok(Expr::Array(
            [1.0f64.into(), 2.0f64.into(), 3.0f64.into(), 4.0f64.into()].into()
        ))
    );
}

#[test]
fn function_contains() {
    let mut tree = Tree::new();
    tree.insert(["matrix", "a"], "foo-bar");
    tree.insert(["matrix", "b"], "bar-baz");
    tree.insert(["matrix", "c"], "[1, 2, 3, 4]");

    let eval = Eval::new(&tree);

    assert_eq!(eval.expr("contains(matrix.a, 'foo')"), Ok(Expr::Bool(true)));
    assert_eq!(
        eval.expr("contains(matrix.a, 'baz')"),
        Ok(Expr::Bool(false))
    );
}

#[test]
fn function_contains_array() {
    let mut tree = Tree::new();
    tree.insert(["matrix", "a"], "foo-bar");
    tree.insert(["matrix", "b"], "bar-baz");
    tree.insert(["matrix", "c"], "[1, 2, 3, 4]");
    tree.insert(["github", "event_name"], "pull_request");

    let eval = Eval::new(&tree);

    assert_eq!(
        eval.expr("contains(matrix.*, 'foo-bar')"),
        Ok(Expr::Bool(true))
    );

    assert_eq!(
        eval.expr("contains(matrix.*, 'baz')"),
        Ok(Expr::Bool(false))
    );

    assert_eq!(
        eval.expr("contains(fromJSON('[\"push\", \"pull_request\"]'), github.event_name)"),
        Ok(Expr::Bool(true))
    );
}

#[test]
fn test_eval() {
    let mut tree = Tree::new();
    tree.insert(["matrix", "a"], "hello");

    let eval = Eval::new(&tree);

    assert_eq!(
        eval.eval("${{ matrix.a }}-world-${{ matrix.b }}-bar"),
        Ok(Cow::Owned(RString::from("hello-world--bar")))
    );

    assert_eq!(
        eval.eval("${{ matrix.a }}-world-${{ matrix.b } }-bar"),
        Ok(Cow::Owned(RString::from(
            "hello-world-${{ matrix.b } }-bar"
        )))
    );
}
