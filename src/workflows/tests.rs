use super::*;

#[test]
fn matrix_test() {
    let mut matrix = Matrix::new();
    matrix.insert("a", "1");
    matrix.insert("b", "2");

    let env = BTreeMap::new();

    let eval = Eval::new().with_env(&env).with_matrix(&matrix);

    assert!(eval.test("matrix.a == '1'").unwrap());
    assert!(eval.test("matrix.a != '2'").unwrap());
    assert!(eval.test("matrix.a != matrix.b").unwrap());
    assert!(!eval.test("matrix.a == matrix.b").unwrap());
    assert!(eval
        .test("matrix.a == matrix.b || matrix.a != matrix.b")
        .unwrap());
}

#[test]
fn or_test() {
    let mut matrix = Matrix::new();
    matrix.insert("bar", "right");

    let eval = Eval::new().with_matrix(&matrix);

    assert_eq!(
        eval.expr("matrix.foo || matrix.bar").unwrap(),
        Expr::String(Cow::Borrowed("right"))
    );
}

#[test]
fn and_test() {
    let mut matrix = Matrix::new();
    matrix.insert("foo", "wrong");
    matrix.insert("bar", "right");

    let eval = Eval::new().with_matrix(&matrix);

    assert_eq!(
        eval.expr("matrix.foo && matrix.bar").unwrap(),
        Expr::String(Cow::Borrowed("right"))
    );

    assert_eq!(eval.expr("matrix.baz && matrix.bar").unwrap(), Expr::Null);
}

#[test]
fn group() {
    let mut matrix = Matrix::new();
    matrix.insert("foo", "wrong");
    matrix.insert("bar", "right");

    let eval = Eval::new().with_matrix(&matrix);

    assert_eq!(
        eval.expr("${{ matrix.foo }} && matrix.bar").unwrap(),
        Expr::String(Cow::Borrowed("right"))
    );

    assert_eq!(eval.expr("matrix.baz && matrix.bar").unwrap(), Expr::Null);
}

#[test]
fn not() {
    let mut matrix = Matrix::new();
    matrix.insert("foo", "");
    matrix.insert("bar", "right");

    let eval = Eval::new().with_matrix(&matrix);

    assert_eq!(eval.expr("!matrix.foo").unwrap(), Expr::Bool(true));
    assert_eq!(eval.expr("!matrix.bar").unwrap(), Expr::Bool(false));
    assert_eq!(eval.expr("!matrix.baz").unwrap(), Expr::Bool(true));
}
