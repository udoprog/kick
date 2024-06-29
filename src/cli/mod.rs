macro_rules! with_repos {
    ($c:expr, $what:expr, $hint:expr, |$cx:ident, $repo:ident| $block:expr $(,)?) => {
        let mut good = $crate::repo_sets::RepoSet::default();
        let mut bad = $crate::repo_sets::RepoSet::default();

        for $repo in $c.repos() {
            let span = tracing::info_span!(
                "repo",
                source = $repo.source().to_string(),
                path = $c.to_path($repo.path()).display().to_string()
            );
            let _span = span.enter();

            if $repo.is_disabled() {
                tracing::trace!("Skipping disabled");
                continue;
            }

            let $cx = &*$c;
            let result = $block;

            tracing::trace!("Running `{}`", $what);

            if let Err(error) = ::anyhow::Context::with_context(result, $cx.context($repo)) {
                tracing::error!("{error}");

                for cause in error.chain().skip(1) {
                    tracing::error!("Caused by: {}", cause);
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

pub(crate) mod changes;
pub(crate) mod check;
pub(crate) mod compress;
pub(crate) mod deb;
pub(crate) mod define;
pub(crate) mod github_action;
pub(crate) mod github_release;
pub(crate) mod msi;
pub(crate) mod msrv;
mod output;
pub(crate) mod publish;
pub(crate) mod rpm;
pub(crate) mod run;
pub(crate) mod set;
pub(crate) mod status;
pub(crate) mod update;
pub(crate) mod upgrade;
pub(crate) mod version;
