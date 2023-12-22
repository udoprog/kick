use std::pin::pin;

use anyhow::{ensure, Context, Result};
use bytes::Bytes;
use futures_core::Stream;
use reqwest::{header, Method, StatusCode};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt};
use url::Url;

use crate::env::SecretString;

const UPLOADS_URL: &str = "https://uploads.github.com";
const API_URL: &str = "https://api.github.com";
static ACCEPT: header::HeaderValue = header::HeaderValue::from_static("application/json");
static OCTET_STREAM: header::HeaderValue =
    header::HeaderValue::from_static("application/octet-stream");

/// A github client.
pub(crate) struct Client {
    uploads_url: Url,
    url: Url,
    token: SecretString,
    client: reqwest::Client,
}

impl Client {
    pub(crate) fn new(token: SecretString) -> Result<Self> {
        let uploads_url = Url::parse(UPLOADS_URL)?;
        let url = Url::parse(API_URL)?;

        let client = reqwest::Client::builder()
            .user_agent(&crate::USER_AGENT)
            .build()?;

        Ok(Self {
            uploads_url,
            url,
            token,
            client,
        })
    }

    /// Fetch all releases related to a repository.
    pub(crate) fn releases(&self, owner: &str, repo: &str) -> Result<Releases> {
        let mut url = self.url.clone();

        url.path_segments_mut()
            .ok()
            .context("path")?
            .extend(&["repos", owner, repo, "releases"]);

        Ok(Releases { url, page: 0 })
    }

    /// Get an existing git reference.
    pub(crate) async fn git_ref_get(
        &self,
        owner: &str,
        repo: &str,
        r#ref: &str,
    ) -> Result<Option<Reference>> {
        let mut url = self.url.clone();

        url.path_segments_mut()
            .ok()
            .context("path")?
            .extend(&["repos", owner, repo, "git", "refs", r#ref]);

        let req = self.request(Method::GET, url.clone()).build()?;

        let res = self.client.execute(req).await?;

        if res.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }

        ensure!(res.status().is_success(), res.status());
        let reference = res.json().await?;
        Ok(Some(reference))
    }

    /// Update a git reference.
    pub(crate) async fn git_ref_update(
        &self,
        owner: &str,
        repo: &str,
        r#ref: &str,
        sha: &str,
        force: bool,
    ) -> Result<Reference> {
        #[derive(Debug, Serialize)]
        struct Request<'a> {
            sha: &'a str,
            force: bool,
        }

        let mut url = self.url.clone();

        url.path_segments_mut()
            .ok()
            .context("path")?
            .extend(&["repos", owner, repo, "git", "refs", r#ref]);

        let body = Request { sha, force };

        let req = self
            .request(Method::PATCH, url.clone())
            .json(&body)
            .build()?;

        let res = self.client.execute(req).await?;

        ensure!(res.status().is_success(), res.status());
        let update = res.json().await?;
        Ok(update)
    }

    /// Create a git reference.
    pub(crate) async fn git_ref_create(
        &self,
        owner: &str,
        repo: &str,
        r#ref: &str,
        sha: &str,
    ) -> Result<Reference> {
        #[derive(Debug, Serialize)]
        struct Request<'a> {
            sha: &'a str,
            r#ref: &'a str,
        }

        let mut url = self.url.clone();

        url.path_segments_mut()
            .ok()
            .context("path")?
            .extend(&["repos", owner, repo, "git", "refs"]);

        let body = Request { sha, r#ref };

        let req = self
            .request(Method::POST, url.clone())
            .json(&body)
            .build()?;

        let res = self.client.execute(req).await?;

        ensure!(res.status().is_success(), res.status());
        let update = res.json().await?;
        Ok(update)
    }

    #[allow(unused)]
    pub(crate) async fn delete_release(&self, owner: &str, repo: &str, id: u64) -> Result<()> {
        let mut url = self.url.clone();

        url.path_segments_mut().ok().context("path")?.extend(&[
            "repos",
            owner,
            repo,
            "releases",
            &id.to_string(),
        ]);

        let req = self.request(Method::DELETE, url).build()?;
        let res = self.client.execute(req).await?;

        ensure!(res.status().is_success(), res.status());
        Ok(())
    }

    /// Create a GitHub release.
    pub(crate) async fn create_release(
        &self,
        owner: &str,
        repo: &str,
        tag_name: &str,
        target_commitish: &str,
        name: &str,
        body: Option<&str>,
        prerelease: bool,
        draft: bool,
    ) -> Result<Release> {
        #[derive(Serialize)]
        struct Request<'a> {
            tag_name: &'a str,
            target_commitish: &'a str,
            name: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            body: Option<&'a str>,
            draft: bool,
            prerelease: bool,
            generate_release_notes: bool,
        }

        let mut url = self.url.clone();

        url.path_segments_mut()
            .ok()
            .context("path")?
            .extend(&["repos", owner, repo, "releases"]);

        let request = Request {
            tag_name,
            target_commitish,
            name,
            body,
            prerelease,
            draft,
            generate_release_notes: false,
        };

        let req = self.request(Method::POST, url).json(&request).build()?;
        let res = self.client.execute(req).await?;

        ensure!(res.status().is_success(), res.status());
        let update = res.json().await?;
        Ok(update)
    }

    /// Create a GitHub release.
    pub(crate) async fn update_release(
        &self,
        owner: &str,
        repo: &str,
        id: u64,
        tag_name: &str,
        target_commitish: &str,
        name: &str,
        body: Option<&str>,
        prerelease: bool,
        draft: bool,
    ) -> Result<Release> {
        #[derive(Serialize)]
        struct Request<'a> {
            tag_name: &'a str,
            target_commitish: &'a str,
            name: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            body: Option<&'a str>,
            draft: bool,
            prerelease: bool,
            generate_release_notes: bool,
        }

        let mut url = self.url.clone();

        url.path_segments_mut().ok().context("path")?.extend(&[
            "repos",
            owner,
            repo,
            "releases",
            &id.to_string(),
        ]);

        let request = Request {
            tag_name,
            target_commitish,
            name,
            body,
            prerelease,
            draft,
            generate_release_notes: false,
        };

        let req = self.request(Method::PATCH, url).json(&request).build()?;
        let res = self.client.execute(req).await?;

        ensure!(res.status().is_success(), res.status());
        let update = res.json().await?;
        Ok(update)
    }

    /// Upload a release asset.
    pub(crate) async fn upload_release_asset<I>(
        &self,
        owner: &str,
        repo: &str,
        id: u64,
        name: &str,
        input: I,
        len: u64,
    ) -> Result<()>
    where
        I: 'static + AsyncRead + Send + Sync,
    {
        let mut url = self.uploads_url.clone();

        url.path_segments_mut().ok().context("path")?.extend(&[
            "repos",
            owner,
            repo,
            "releases",
            &id.to_string(),
            "assets",
        ]);

        url.query_pairs_mut().append_pair("name", name);

        let body = reqwest::Body::wrap_stream(stream_body(input));

        let req = self
            .request(Method::POST, url.clone())
            .header(header::CONTENT_TYPE, &OCTET_STREAM)
            .header(header::CONTENT_LENGTH, len)
            .body(body)
            .build()?;

        let res = self.client.execute(req).await?;

        ensure!(res.status().is_success(), res.status());
        Ok(())
    }

    /// Delete a release asset.
    pub(crate) async fn delete_release_asset(
        &self,
        owner: &str,
        repo: &str,
        id: u64,
    ) -> Result<()> {
        let mut url = self.url.clone();

        url.path_segments_mut().ok().context("path")?.extend(&[
            "repos",
            owner,
            repo,
            "releases",
            "assets",
            &id.to_string(),
        ]);

        let req = self.request(Method::DELETE, url).build()?;
        let res = self.client.execute(req).await?;

        ensure!(res.status().is_success(), res.status());
        Ok(())
    }

    /// Fetch the next page of results.
    pub(crate) async fn next_page<T>(&self, paged: &mut T) -> Result<Option<Vec<T::Item>>>
    where
        T: ?Sized + Paginate,
    {
        let mut url = paged.url().clone();
        let page = paged.next_page();
        url.query_pairs_mut().append_pair("page", &page.to_string());

        let req = self.request(Method::GET, url.clone()).build()?;

        let res = self.client.execute(req).await?;

        ensure!(res.status().is_success(), res.status());
        let page: Vec<T::Item> = res.json().await?;

        if page.is_empty() {
            return Ok(None);
        }

        Ok(Some(page))
    }

    fn request(&self, method: Method, url: Url) -> reqwest::RequestBuilder {
        self.client
            .request(method, url.clone())
            .header(header::ACCEPT, &ACCEPT)
            .header(
                header::AUTHORIZATION,
                format!("Bearer {}", self.token.as_secret()),
            )
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct Asset {
    pub(crate) name: String,
    pub(crate) id: u64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Release {
    pub(crate) id: u64,
    pub(crate) tag_name: String,
    pub(crate) draft: bool,
    pub(crate) prerelease: bool,
    #[serde(default)]
    pub(crate) assets: Vec<Asset>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Object {
    pub(crate) sha: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Reference {
    pub(crate) object: Object,
}

#[derive(Deserialize)]
pub(crate) struct Releases {
    url: Url,
    page: usize,
}

pub(crate) trait Paginate {
    type Item: DeserializeOwned;

    fn url(&self) -> &Url;

    fn next_page(&mut self) -> usize;
}

impl Paginate for Releases {
    type Item = Release;

    fn url(&self) -> &Url {
        &self.url
    }

    fn next_page(&mut self) -> usize {
        let page = self.page + 1;
        self.page = page;
        page
    }
}

/// Helper method to construct a stream out of a body.
fn stream_body<I>(input: I) -> impl Stream<Item = Result<Bytes>>
where
    I: 'static + AsyncRead + Send + Sync,
{
    async_stream::try_stream! {
        let mut input = pin!(input);
        let mut buf = [0u8; 8192];

        loop {
            let n = input.read(&mut buf).await?;

            if n == 0 {
                break;
            }

            yield Bytes::copy_from_slice(&buf[..n]);
        }
    }
}
