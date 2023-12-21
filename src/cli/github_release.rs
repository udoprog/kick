use std::env;

use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;
use relative_path::RelativePathBuf;
use tokio::fs::File;

use crate::ctxt::Ctxt;
use crate::glob::Glob;
use crate::model::Repo;
use crate::octokit::Client;
use crate::release::ReleaseOpts;

#[derive(Default, Debug, Clone, Parser)]
pub(crate) struct Opts {
    #[clap(flatten)]
    release: ReleaseOpts,
    /// SHA of Github commit.
    #[arg(long, value_name = "sha")]
    sha: Option<String>,
    /// Get sha from the specified environment variable.
    #[arg(long, value_name = "env")]
    sha_from_env: Option<String>,
    /// The body of the release.
    #[arg(long, value_name = "text")]
    body: Option<String>,
    /// Indicates if the new release is a draft.
    #[arg(long)]
    draft: bool,
    /// Pattern of release assets to upload.
    #[arg(long, value_name = "glob")]
    upload: Vec<RelativePathBuf>,
    /// Get details from the GitHub action context.
    #[arg(long)]
    github_action: bool,
}

pub(crate) async fn entry(cx: &mut Ctxt<'_>, opts: &Opts) -> Result<()> {
    let mut opts = opts.clone();

    if opts.github_action {
        opts.sha_from_env = Some("GITHUB_SHA".into());
    }

    if opts.sha.is_none() {
        if let Some(env) = opts.sha_from_env.as_deref() {
            if let Ok(sha) = env::var("GITHUB_SHA") {
                tracing::info!("Using sha from {env}={sha}");
                opts.sha = Some(sha);
            }
        }
    }

    let version = opts.release.version(cx.env)?;

    let Some(token) = &cx.env.github_token else {
        bail!("Missing access token");
    };

    let Some(sha) = &opts.sha else {
        bail!("Missing SHA to update the commit to, either provide --sha or set GITHUB_SHA and use --github-action");
    };

    let client = Client::new(token.clone())?;
    let name = version.to_string();
    let prerelease = version.is_pre();

    with_repos!(
        cx,
        "Github Release",
        format_args!("github-release: {opts:?}"),
        |cx, repo| { github_publish(cx, &opts, repo, &client, &name, sha, prerelease).await }
    );

    Ok(())
}

#[tracing::instrument(skip_all, fields(source = ?repo.source(), path = repo.path().as_str()))]
async fn github_publish(
    cx: &Ctxt<'_>,
    opts: &Opts,
    repo: &Repo,
    client: &Client,
    name: &str,
    sha: &str,
    prerelease: bool,
) -> Result<()> {
    let Some(path) = repo.repo() else {
        bail!("Repo is not a github repo");
    };

    tracing::info! {
        owner = path.owner,
        name = path.name,
        name,
        sha,
        name,
        body = opts.body.as_deref(),
        prerelease,
        draft = opts.draft,
        "Publishing"
    };

    let mut releases = client.releases(path.owner, path.name)?;

    let id = 'out: {
        while let Some(page) = client
            .next_page(&mut releases)
            .await
            .context("Downloading releases")?
        {
            for release in page {
                if release.tag_name == name {
                    break 'out Some(release.id);
                }
            }
        }

        None
    };

    if let Some(id) = id {
        tracing::info!("Deleting release '{}' (id: {id})", name);
        client
            .delete_release(path.owner, path.name, id)
            .await
            .context("Deleting old release")?;
    }

    if cx.env.github_tag() != Some(name) {
        tracing::info!("Trying to update tag '{}'", name);
        let r#ref = format!("tags/{}", name);
        let update = client
            .git_ref_update(path.owner, path.name, &r#ref, sha, true)
            .await
            .with_context(|| anyhow!("Updating tag '{}'", r#ref))?;

        if update.is_none() {
            tracing::info!("Creating tag '{}'", name);
            let r#ref = format!("refs/tags/{}", name);

            client
                .git_ref_create(path.owner, path.name, &r#ref, sha)
                .await
                .with_context(|| anyhow!("Creating tag '{}'", r#ref))?;
        }
    }

    tracing::info!("Creating release '{}'", name);
    let release = client
        .create_release(
            path.owner,
            path.name,
            name,
            sha,
            name,
            opts.body.as_deref(),
            prerelease,
            opts.draft,
        )
        .await?;

    for upload in &opts.upload {
        let root = cx.to_path(repo.path());

        let glob = Glob::new(&root, &upload);

        for m in glob.matcher() {
            let m = m?;

            let Some(name) = m.file_name() else {
                tracing::warn!("Could not determine file name: {m}");
                continue;
            };

            let m = m.to_path(&root);

            tracing::info!("Uploading asset {}", m.display());

            let meta = tokio::fs::metadata(&m)
                .await
                .with_context(|| m.display().to_string())?;

            let f = File::open(&m)
                .await
                .with_context(|| m.display().to_string())?;

            client
                .upload_release_asset(path.owner, path.name, release.id, name, f, meta.len())
                .await
                .with_context(|| anyhow!("Uploading asset {}", m.display()))?;
        }
    }

    Ok(())
}
