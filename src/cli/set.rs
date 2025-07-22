use anyhow::Result;
use clap::Parser;

use crate::ctxt::Ctxt;
use crate::repo_sets::RepoSet;

#[derive(Default, Debug, Parser)]
pub(crate) struct Opts {
    /// Construct a new set with the given name.
    #[arg(name = "new-set", value_name = "set")]
    set: String,
}

pub(crate) fn entry(cx: &mut Ctxt<'_>, opts: &Opts) -> Result<()> {
    let mut set = RepoSet::default();

    for repo in cx.repos() {
        set.insert(repo);
    }

    let hint = format!("built set: {opts:?}");
    cx.sets.save(&opts.set, set, &hint);
    Ok(())
}
