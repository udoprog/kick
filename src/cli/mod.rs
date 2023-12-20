macro_rules! with_repos {
    ($c:expr, $what:expr, $hint:expr, |$cx:ident, $repo:ident| $block:expr $(,)?) => {
        let mut good = $crate::repo_sets::RepoSet::default();
        let mut bad = $crate::repo_sets::RepoSet::default();

        for $repo in $c.repos() {
            if $repo.is_disabled() {
                tracing::trace!(repo = ?$repo.path(), "Skipping disabled");
                continue;
            }

            let $cx = &*$c;
            let result = $block;

            tracing::trace!(repo = ?$repo.path(), "Running `{}`", $what);

            if let Err(error) = ::anyhow::Context::with_context(result, $cx.context($repo)) {
                tracing::error!(repo = ?$repo.path(), "Failed `{}`", $what);

                for cause in error.chain() {
                    tracing::error!(repo = ?$repo.path(), "Caused by: {}", cause);
                }

                $repo.set_error();
                bad.insert($repo);
            } else {
                $repo.set_success();
                good.insert($repo);
            }
        }

        $c.sets.save("good", good, &$hint);
        $c.sets.save("bad", bad, &$hint);
    };
}

pub(crate) mod check;
pub(crate) mod compress;
pub(crate) mod r#for;
pub(crate) mod msi;
pub(crate) mod msrv;
pub(crate) mod publish;
pub(crate) mod rpm;
pub(crate) mod set;
pub(crate) mod status;
pub(crate) mod upgrade;
pub(crate) mod version;
