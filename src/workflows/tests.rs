use super::*;

#[test]
fn matrix_test() {
    let mut matrix = Matrix::new();
    matrix.insert("a", "1");
    matrix.insert("b", "2");

    let env = BTreeMap::new();

    let eval = Eval::new().with_env(&env).with_matrix(&matrix);

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
    let mut matrix = Matrix::new();
    matrix.insert("bar", "right");

    let eval = Eval::new().with_matrix(&matrix);

    assert_eq!(
        eval.expr("matrix.foo || matrix.bar"),
        Ok(Expr::String(Cow::Borrowed("right")))
    );
}

#[test]
fn and_test() {
    let mut matrix = Matrix::new();
    matrix.insert("foo", "wrong");
    matrix.insert("bar", "right");

    let eval = Eval::new().with_matrix(&matrix);

    assert_eq!(
        eval.expr("matrix.foo && matrix.bar"),
        Ok(Expr::String(Cow::Borrowed("right")))
    );

    assert_eq!(eval.expr("matrix.baz && matrix.bar"), Ok(Expr::Null));
}

#[test]
fn group() {
    let mut matrix = Matrix::new();
    matrix.insert("foo", "wrong");
    matrix.insert("bar", "right");

    let eval = Eval::new().with_matrix(&matrix);

    assert_eq!(
        eval.expr("${{ matrix.foo }} && matrix.bar"),
        Ok(Expr::String(Cow::Borrowed("right")))
    );

    assert_eq!(eval.expr("matrix.baz && matrix.bar"), Ok(Expr::Null));
}

#[test]
fn not() {
    let mut matrix = Matrix::new();
    matrix.insert("foo", "");
    matrix.insert("bar", "right");

    let eval = Eval::new().with_matrix(&matrix);

    assert_eq!(eval.expr("!matrix.foo"), Ok(Expr::Bool(true)));
    assert_eq!(eval.expr("!matrix.bar"), Ok(Expr::Bool(false)));
    assert_eq!(eval.expr("!matrix.baz"), Ok(Expr::Bool(true)));
}

#[test]
fn lazy_expansion() {
    let mut matrix = Matrix::new();
    matrix.insert("ref", "refs/heads/main");
    matrix.insert("ref2", "refs/heads/feature");
    let eval = Eval::new().with_matrix(&matrix);

    assert_eq!(
        eval.expr(
            r#"
        matrix.ref == 'refs/heads/main' && 'value_for_main_branch' ||
        'value_for_other_branches'
        "#
        ),
        Ok(Expr::String(Cow::Borrowed("value_for_main_branch")))
    );

    assert_eq!(
        eval.expr(
            r#"
        matrix.ref2 == 'refs/heads/main' && 'value_for_main_branch' ||
        'value_for_other_branches'
        "#
        ),
        Ok(Expr::String(Cow::Borrowed("value_for_other_branches")))
    );
}

#[test]
fn comparisons() {
    let eval = Eval::new();
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
    let mut matrix = Matrix::new();
    matrix.insert("a", "first");
    matrix.insert("b", "second");
    let eval = Eval::new().with_matrix(&matrix);

    let expected = Expr::Array(["first".into(), "second".into()].into());
    assert_eq!(eval.expr("matrix.*"), Ok(expected));
}

#[test]
fn function() {
    let mut matrix = Matrix::new();
    matrix.insert("a", "true");
    matrix.insert("b", "false");
    matrix.insert("c", "[1, 2, 3, 4]");
    let functions = default_functions();
    let eval = Eval::new().with_matrix(&matrix).with_functions(&functions);
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
