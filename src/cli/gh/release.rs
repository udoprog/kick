use std::collections::HashMap;
use std::env;

use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;
use relative_path::RelativePathBuf;
use tokio::fs::File;

use crate::cli::WithReposAsync;
use crate::ctxt::Ctxt;
use crate::glob::Glob;
use crate::model::Repo;
use crate::octokit;
use crate::release::ReleaseOpts;

#[derive(Default, Debug, Clone, Parser)]
pub(super) struct Opts {
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
    /// Delete any existing releases.
    ///
    /// If this is not specified, then an existing release will instead be
    /// updated if necessary.
    #[arg(long)]
    delete: bool,
    /// Delete any existing assets before uploading new ones.
    #[arg(long)]
    delete_assets: bool,
    /// Get details from the GitHub action context.
    #[arg(long)]
    github_action: bool,
}

pub(super) async fn entry(
    opts: &Opts,
    with_repos: impl WithReposAsync<'_>,
    client: &octokit::Client,
) -> Result<()> {
    let sha_from_env = if opts.github_action {
        Some("GITHUB_SHA")
    } else {
        opts.sha_from_env.as_deref()
    };

    let local_env;

    let env_sha = 'done: {
        if let Some(sha) = opts.sha.as_deref() {
            break 'done Some(sha);
        };

        let Some(env) = sha_from_env else {
            break 'done None;
        };

        let Ok(s) = env::var(env) else {
            break 'done None;
        };

        local_env = s;

        tracing::info!("Using SHA from {env}={local_env}");
        Some(&local_env)
    };

    with_repos
        .run_async(
            "Github API (release)",
            format_args!("Github API (release): {opts:?}"),
            async |cx, repo| run(cx, repo, opts, client, env_sha).await,
            |_| Ok(()),
        )
        .await?;

    Ok(())
}

#[tracing::instrument(skip_all)]
async fn run(
    cx: &Ctxt<'_>,
    repo: &Repo,
    opts: &Opts,
    client: &octokit::Client,
    env_sha: Option<&str>,
) -> Result<()> {
    let version = opts.release.version(cx, repo)?;
    let name = version.to_string();
    let prerelease = version.is_pre();

    let Some(path) = repo.repo() else {
        bail!("Repo is not a github repo");
    };

    let git_sha;

    let sha = match env_sha {
        Some(sha) => sha,
        None => {
            let git = cx.require_git()?;
            let dir = cx.to_path(repo.path());
            git_sha = git.rev_parse(dir, "HEAD")?;
            tracing::info!("Using HEAD commit from git (sha: {git_sha})");
            &git_sha
        }
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
        "Creating release"
    };

    if cx.env.github_tag() == Some(name.as_str()) {
        tracing::info!("Not updating '{name}' which is being built through GITHUB_REF");
    } else {
        let r#ref = format!("tags/{}", name);

        let existing = client
            .git_ref_get(path.owner, path.name, &r#ref)
            .await
            .with_context(|| anyhow!("Getting tag '{}'", r#ref))?;

        if let Some(existing) = existing {
            if existing.object.sha != sha {
                tracing::info!("Updating tag '{}' (sha: {sha})", name);
                let r#ref = format!("tags/{}", name);

                client
                    .git_ref_update(path.owner, path.name, &r#ref, sha, true)
                    .await
                    .with_context(|| anyhow!("Updating tag '{}'", r#ref))?;
            }
        } else {
            tracing::info!("Creating tag '{}' (sha: {sha})", name);
            let r#ref = format!("refs/tags/{}", name);

            client
                .git_ref_create(path.owner, path.name, &r#ref, sha)
                .await
                .with_context(|| anyhow!("Creating tag '{}'", r#ref))?;
        }
    }

    let releases = client.releases(path.owner, path.name).await?;

    let mut release = 'out: {
        let Some(mut releases) = releases else {
            break 'out None;
        };

        while let Some(page) = client
            .next_page(&mut releases)
            .await
            .context("Downloading releases")?
        {
            for release in page {
                if release.tag_name == name {
                    break 'out Some(release);
                }
            }
        }

        None
    };

    if opts.delete {
        if let Some(release) = release.take() {
            tracing::info!("Deleting release '{name}' (id: {})", release.id);

            client
                .delete_release(path.owner, path.name, release.id)
                .await
                .with_context(|| anyhow!("Deleting release '{name}'"))?;
        }
    }

    let release = if let Some(release) = release {
        if release.draft != opts.draft
            || release.prerelease != prerelease
            || release.target_commitish != sha
        {
            tracing::info!("Updating existing release '{name}' (id: {})", release.id);
            client
                .update_release(
                    path.owner,
                    path.name,
                    release.id,
                    &name,
                    sha,
                    &name,
                    opts.body.as_deref(),
                    prerelease,
                    opts.draft,
                )
                .await?
        } else {
            release
        }
    } else {
        tracing::info!("Creating release '{name}'");
        client
            .create_release(
                path.owner,
                path.name,
                &name,
                sha,
                &name,
                opts.body.as_deref(),
                prerelease,
                opts.draft,
            )
            .await?
    };

    let mut existing = HashMap::new();

    if opts.delete_assets {
        for asset in &release.assets {
            tracing::info!("Deleting asset '{}' (id: {})", asset.name, asset.id);

            client
                .delete_release_asset(path.owner, path.name, asset.id)
                .await
                .with_context(|| anyhow!("Deleting asset {}", asset.name))?;
        }
    } else {
        for asset in &release.assets {
            existing.insert(asset.name.clone(), asset.id);
        }
    }

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

            let meta = tokio::fs::metadata(&m)
                .await
                .with_context(|| m.display().to_string())?;

            let f = File::open(&m)
                .await
                .with_context(|| m.display().to_string())?;

            if let Some(&asset_id) = existing.get(name) {
                tracing::info!("Deleting existing asset {}", m.display());
                client
                    .delete_release_asset(path.owner, path.name, asset_id)
                    .await?;
            }

            tracing::info!("Uploading new asset {}", m.display());

            client
                .upload_release_asset(path.owner, path.name, release.id, name, f, meta.len())
                .await
                .with_context(|| anyhow!("Uploading asset {}", m.display()))?;
        }
    }

    Ok(())
}
