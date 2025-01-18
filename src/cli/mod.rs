macro_rules! async_with_repos {
    ($c:expr, $what:expr, $hint:expr, $parallelism:expr, async |$cx:ident, $repo:ident| $block:expr, |$report:pat_param| $report_block:expr $(,)?) => {
        let mut good = $crate::repo_sets::RepoSet::default();
        let mut bad = $crate::repo_sets::RepoSet::default();

        {
            let mut futures = futures_util::stream::FuturesUnordered::new();
            let mut count = 0;

            let mut it = $c.repos();

            while !$c.is_terminated() {
                while count < $parallelism {
                    let Some($repo) = it.next() else {
                        break;
                    };

                    let span = ::tracing::info_span!(
                        "repo",
                        source = $repo.source().to_string(),
                        path = $c.to_path($repo.path()).display().to_string()
                    );

                    let $cx = &*$c;

                    let result = async {
                        tracing::trace!("Running `{}`", $what);
                        let output = $block;
                        Ok::<_, ::anyhow::Error>(output)
                    };

                    futures.push(async move {
                        let result = ::tracing::Instrument::instrument(result, span.clone()).await;
                        (result, $repo, span)
                    });

                    count += 1;
                }

                let Some((result, repo, span)) =
                    ::futures_util::stream::StreamExt::next(&mut futures).await
                else {
                    break;
                };

                count -= 1;

                let _span = span.enter();

                let result = match result {
                    Ok($report) => $report_block,
                    Err(error) => Err(error),
                };

                match ::anyhow::Context::with_context(result, $c.context(repo)) {
                    Ok(()) => {
                        repo.set_success();
                        good.insert(repo);
                    }
                    Err(error) => {
                        tracing::error!("{error}");

                        for cause in error.chain().skip(1) {
                            tracing::error!("Caused by: {}", cause);
                        }

                        repo.set_error();
                        bad.insert(repo);
                    }
                }
            }
        }

        $c.sets.save("good", good, &$hint);
        $c.sets.save("bad", bad, &$hint);
    };
}

macro_rules! with_repos {
    ($c:expr, $what:expr, $hint:expr, |$cx:ident, $repo:ident| $block:expr $(,)?) => {
        let mut good = $crate::repo_sets::RepoSet::default();
        let mut bad = $crate::repo_sets::RepoSet::default();

        for $repo in $c.repos() {
            if $c.is_terminated() {
                break;
            }

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
pub(crate) mod workflows;
