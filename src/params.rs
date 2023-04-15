/// Badge building parameters.
#[derive(Debug, Clone)]
pub(crate) struct Params<'a> {
    pub(crate) repo: &'a str,
    pub(crate) name: &'a str,
}
