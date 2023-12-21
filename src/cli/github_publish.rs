use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;
use relative_path::RelativePathBuf;
use tokio::fs::File;
use tokio::time::sleep;

use crate::ctxt::Ctxt;
use crate::glob::Glob;
use crate::model::Repo;
use crate::octokit::Client;

#[derive(Default, Debug, Parser)]
pub(crate) struct Opts {
    /// Name of release to create.
    #[arg(long)]
    name: String,
    /// SHA of Github commit.
    #[arg(long)]
    sha: String,
    /// Provide an access token to use to access the API.
    #[arg(long, value_name = "token")]
    token: Option<String>,
    /// The body of the release.
    #[arg(long, value_name = "text")]
    body: Option<String>,
    /// Indicates if the new release is a prerelease.
    #[arg(long)]
    prerelease: bool,
    /// Indicates if the new release is a draft.
    #[arg(long)]
    draft: bool,
    /// Pattern of release assets to upload.
    #[arg(long)]
    upload: Option<RelativePathBuf>,
}

pub(crate) async fn entry(cx: &mut Ctxt<'_>, opts: &Opts) -> Result<()> {
    let Some(token) = opts.token.as_deref().or(cx.github_auth.as_deref()) else {
        bail!("Missing access token");
    };

    let client = Client::new(token.to_owned())?;

    with_repos!(
        cx,
        "github-publish",
        format_args!("github-publish: {opts:?}"),
        |cx, repo| { github_publish(cx, opts, repo, &client).await }
    );

    Ok(())
}

#[tracing::instrument(skip_all, fields(source = ?repo.source(), path = repo.path().as_str()))]
async fn github_publish(cx: &Ctxt<'_>, opts: &Opts, repo: &Repo, client: &Client) -> Result<()> {
    let Some(path) = repo.repo() else {
        bail!("Repo is not a github repo");
    };

    let mut releases = client.releases(path.owner, path.name)?;

    let id = 'out: {
        while let Some(page) = client
            .next_page(&mut releases)
            .await
            .context("Downloading releases")?
        {
            for release in page {
                if release.tag_name == opts.name {
                    break 'out Some(release.id);
                }
            }
        }

        None
    };

    if let Some(id) = id {
        tracing::info!("Deleting release '{}' (id: {id})", opts.name);
        client
            .delete_release(path.owner, path.name, id)
            .await
            .context("Deleting old release")?;
    }

    let r#ref = format!("tags/{}", opts.name);

    tracing::info!("Trying to update tag '{}'", opts.name);
    let update = client
        .git_ref_update(path.owner, path.name, &r#ref, &opts.sha, true)
        .await
        .with_context(|| anyhow!("Updating tag '{}'", r#ref))?;

    let update = match update {
        Some(update) => update,
        None => {
            tracing::info!("Creating tag '{}'", opts.name);
            let r#ref = format!("refs/tags/{}", opts.name);

            client
                .git_ref_create(path.owner, path.name, &r#ref, &opts.sha)
                .await
                .with_context(|| anyhow!("Updating tag '{}'", r#ref))?
        }
    };

    let body = opts.body.as_deref().unwrap_or_default();

    tracing::info!("Creating release '{}'", opts.name);

    let release = client
        .create_release(
            path.owner,
            path.name,
            &opts.name,
            &update.object.sha,
            &opts.name,
            body,
            opts.prerelease,
            opts.draft,
        )
        .await?;

    if let Some(upload) = opts.upload.as_deref() {
        let root = cx.to_path(repo.path());

        let glob = Glob::new(&root, &upload);

        for m in glob.matcher() {
            let m = m?;

            let Some(name) = m.file_name() else {
                tracing::warn!("Could not determine file name: {m}");
                continue;
            };

            tracing::info!("Uploading asset '{m}'");
            let m = m.to_path(&root);

            let meta = tokio::fs::metadata(&m)
                .await
                .with_context(|| m.display().to_string())?;

            for _ in 0..10 {
                let m = File::open(&m)
                    .await
                    .with_context(|| m.display().to_string())?;

                let result = client
                    .upload_release_asset(path.owner, path.name, release.id, name, m, meta.len())
                    .await;

                if let Err(error) = result {
                    tracing::warn!("Failed to upload: {}", error);
                    sleep(Duration::from_secs(1)).await;
                    continue;
                }

                break;
            }
        }
    }

    Ok(())
}
