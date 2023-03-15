/// Badge building parameters.
#[derive(Debug, Clone)]
pub(crate) struct Params<'a> {
    pub(crate) repo: &'a str,
    pub(crate) crate_name: &'a str,
}
