use core::fmt;

use crate::ctxt::Ctxt;
use crate::model::Repo;

use anyhow::Result;
use futures_util::stream::StreamExt;
use tracing::Instrument;

/// Run over repos asynchronously with a final report on successful run.
async fn with_repos_async<'repo, T>(
    cx: &mut Ctxt<'repo>,
    what: impl fmt::Display,
    hint: impl fmt::Display,
    parallelism: usize,
    f: impl AsyncFn(&Ctxt<'repo>, &'repo Repo) -> Result<T>,
    mut report_fn: impl FnMut(T) -> Result<()>,
) -> Result<()> {
    let mut good = crate::repo_sets::RepoSet::default();
    let mut bad = crate::repo_sets::RepoSet::default();

    {
        let (cx, what, f) = (&*cx, &what, &f);

        let mut futures = futures_util::stream::FuturesUnordered::new();
        let mut count = 0;

        let mut it = cx.repos();

        loop {
            let done = cx.is_terminated();

            while !done && count < parallelism {
                let Some(repo) = it.next() else {
                    break;
                };

                let span = ::tracing::info_span!(
                    "repo",
                    sources = repo.sources().to_string(),
                    path = cx.to_path(repo.path()).display().to_string()
                );

                futures.push(async move {
                    tracing::trace!("Running `{what}`");
                    let result = f(cx, repo).instrument(span.clone()).await;
                    (result, repo, span)
                });

                count += 1;
            }

            let Some((result, repo, span)) = futures.next().await else {
                break;
            };

            count -= 1;

            let _span = span.enter();

            let result = match result {
                Ok(report) => report_fn(report),
                Err(error) => Err(error),
            };

            match result {
                Ok(()) => {
                    repo.set_success();
                    good.insert(repo);
                }
                Err(error) => {
                    tracing::error!("{error}");

                    for cause in error.chain().skip(1) {
                        tracing::error!("Caused by: {cause}");
                    }

                    repo.set_error();
                    bad.insert(repo);
                }
            }
        }

        for repo in it {
            repo.set_error();
            bad.insert(repo);
        }
    }

    cx.sets.save("good", good, &hint);
    cx.sets.save("bad", bad, &hint);
    Ok(())
}

/// Run over repos.
fn with_repos<'repo, T>(
    cx: &mut Ctxt<'repo>,
    what: impl fmt::Display,
    hint: impl fmt::Display,
    mut f: impl FnMut(&Ctxt<'repo>, &'repo Repo) -> Result<T>,
) -> Result<()> {
    let mut good = crate::repo_sets::RepoSet::default();
    let mut bad = crate::repo_sets::RepoSet::default();

    let mut it = cx.repos();

    loop {
        if cx.is_terminated() {
            break;
        }

        let Some(repo) = it.next() else {
            break;
        };

        let span = tracing::info_span!(
            "repo",
            sources = repo.sources().to_string(),
            path = cx.to_path(repo.path()).display().to_string()
        );
        let _span = span.enter();

        let result = f(cx, repo);

        tracing::trace!("Running `{what}`");

        if let Err(error) = result {
            tracing::error!("{error}");

            for cause in error.chain().skip(1) {
                tracing::error!("Caused by: {cause}");
            }

            repo.set_error();
            bad.insert(repo);
        } else {
            repo.set_success();
            good.insert(repo);
        }
    }

    for repo in it {
        repo.set_error();
        bad.insert(repo);
    }

    cx.sets.save("good", good, &hint);
    cx.sets.save("bad", bad, &hint);

    Ok(())
}

pub(crate) struct WithRepos<'repo> {
    pub(crate) cx: Ctxt<'repo>,
    parallelism: usize,
}

impl<'repo> WithRepos<'repo> {
    #[inline]
    pub(crate) fn new(cx: Ctxt<'repo>, parallelism: usize) -> Self {
        Self { cx, parallelism }
    }
}

impl<'repo> WithRepos<'repo> {
    #[inline]
    pub(crate) fn cx(&self) -> &Ctxt<'repo> {
        &self.cx
    }

    #[inline]
    pub(crate) fn into_cx(self) -> Ctxt<'repo> {
        self.cx
    }

    #[inline]
    pub(crate) fn run(
        &mut self,
        what: impl fmt::Display,
        hint: impl fmt::Display,
        f: impl FnMut(&Ctxt<'repo>, &'repo Repo) -> Result<()>,
    ) -> Result<()> {
        with_repos(&mut self.cx, what, hint, f)
    }

    #[inline]
    pub(crate) async fn run_async<T>(
        &mut self,
        what: impl fmt::Display,
        hint: impl fmt::Display,
        f: impl AsyncFn(&Ctxt<'repo>, &'repo Repo) -> Result<T>,
        report: impl FnMut(T) -> Result<()>,
    ) -> Result<()> {
        with_repos_async(&mut self.cx, what, hint, self.parallelism, f, report).await
    }
}
