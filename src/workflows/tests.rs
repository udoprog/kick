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
