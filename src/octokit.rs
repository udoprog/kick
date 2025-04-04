use std::marker::PhantomData;
use std::pin::pin;

use anyhow::{anyhow, Context, Result};
use bytes::Bytes;
use chrono::{DateTime, Utc};
use futures_core::Stream;
use reqwest::header::HeaderMap;
use reqwest::{header, Method, Response, StatusCode};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt};
use url::Url;

use crate::env::SecretString;

const UPLOADS_URL: &str = "https://uploads.github.com";
const API_URL: &str = "https://api.github.com";
static ACCEPT: header::HeaderValue = header::HeaderValue::from_static("application/json");
static OCTET_STREAM: header::HeaderValue =
    header::HeaderValue::from_static("application/octet-stream");

async fn ensure(res: Response) -> Result<Response> {
    if !res.status().is_success() {
        return Err(anyhow!("{}: {}", res.status(), res.text().await?));
    }

    Ok(res)
}

pub(crate) enum Auth {
    Bearer(SecretString),
    Basic(SecretString),
    None,
}

/// A github client.
pub(crate) struct Client {
    uploads_url: Url,
    url: Url,
    auth: Auth,
    client: reqwest::Client,
}

impl Client {
    pub(crate) fn new(auth: Auth) -> Result<Self> {
        let uploads_url = Url::parse(UPLOADS_URL)?;
        let url = Url::parse(API_URL)?;

        let mut headers = HeaderMap::new();
        headers.insert(
            "X-GitHub-Api-Version",
            header::HeaderValue::from_static("2022-11-28"),
        );

        let client = reqwest::Client::builder()
            .user_agent(&crate::USER_AGENT)
            .default_headers(headers)
            .build()?;

        Ok(Self {
            uploads_url,
            url,
            auth,
            client,
        })
    }

    /// Get the latest release in a repo.
    pub(crate) async fn latest_release(&self, owner: &str, repo: &str) -> Result<Option<Release>> {
        let mut url = self.url.clone();

        url.path_segments_mut()
            .ok()
            .context("path")?
            .extend(["repos", owner, repo, "releases", "latest"]);

        let Some(release) = self.get(&url).await? else {
            return Ok(None);
        };

        Ok(Some(release))
    }

    /// Enable a workflow.
    pub(crate) async fn workflows_enable(
        &self,
        owner: &str,
        repo: &str,
        id: u64,
    ) -> Result<(StatusCode, String)> {
        let mut url = self.url.clone();

        url.path_segments_mut()
            .ok()
            .context("path")?
            .extend(["repos", owner, repo, "actions", "workflows"])
            .extend([id.to_string()])
            .push("enable");

        let req = self.request(Method::PUT, url.clone())?.build()?;
        let res = self.client.execute(req).await?;
        Ok((res.status(), res.text().await?))
    }

    /// Disable a workflow.
    pub(crate) async fn workflows_disable(
        &self,
        owner: &str,
        repo: &str,
        id: u64,
    ) -> Result<(StatusCode, String)> {
        let mut url = self.url.clone();

        url.path_segments_mut()
            .ok()
            .context("path")?
            .extend(["repos", owner, repo, "actions", "workflows"])
            .extend([id.to_string()])
            .push("disable");

        let req = self.request(Method::PUT, url.clone())?.build()?;
        let res = self.client.execute(req).await?;
        Ok((res.status(), res.text().await?))
    }

    /// List available workflows.
    pub(crate) async fn workflows_list(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<Option<Workflows>> {
        let mut url = self.url.clone();

        url.path_segments_mut().ok().context("path")?.extend([
            "repos",
            owner,
            repo,
            "actions",
            "workflows",
        ]);

        let req = self.request(Method::GET, url.clone())?.build()?;

        let res = self.client.execute(req).await?;

        if res.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }

        let res = ensure(res).await?;
        let data = res.json().await?;
        Ok(Some(data))
    }

    /// Get the runs for the given workflow id.
    ///
    /// Returns `None` if the workflow doesn't exist.
    pub(crate) async fn workflow_runs(
        &self,
        owner: &str,
        repo: &str,
        id: &str,
        exclude_pull_requests: bool,
        per_page: Option<usize>,
    ) -> Result<Option<PagedWorkflowRuns>> {
        let mut url = self.url.clone();

        url.path_segments_mut()
            .ok()
            .context("path")?
            .extend(["repos", owner, repo, "actions", "workflows"])
            .extend([format!("{id}.yml")])
            .push("runs");

        {
            let mut query = url.query_pairs_mut();
            query.append_pair("exclude_pull_requests", &exclude_pull_requests.to_string());

            if let Some(per_page) = per_page {
                query.append_pair("per_page ", &per_page.to_string());
            }
        }

        let Some(initial) = self.get(&url).await? else {
            return Ok(None);
        };

        Ok(Some(PagedWorkflowRuns::new(url, initial)))
    }

    /// Fetch all releases related to a repository.
    pub(crate) async fn releases(&self, owner: &str, repo: &str) -> Result<Option<Paged<Release>>> {
        let mut url = self.url.clone();

        url.path_segments_mut()
            .ok()
            .context("path")?
            .extend(["repos", owner, repo, "releases"]);

        let Some(initial) = self.get(&url).await? else {
            return Ok(None);
        };

        Ok(Some(Paged::new(url, initial)))
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
            .extend(["repos", owner, repo, "git", "refs", r#ref]);

        let req = self.request(Method::GET, url.clone())?.build()?;

        let res = self.client.execute(req).await?;

        if res.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }

        let res = ensure(res).await?;
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
            .extend(["repos", owner, repo, "git", "refs", r#ref]);

        let body = Request { sha, force };

        let req = self
            .request(Method::PATCH, url.clone())?
            .json(&body)
            .build()?;

        let res = self.client.execute(req).await?;

        let res = ensure(res).await?;
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
            .extend(["repos", owner, repo, "git", "refs"]);

        let body = Request { sha, r#ref };

        let req = self
            .request(Method::POST, url.clone())?
            .json(&body)
            .build()?;

        let res = self.client.execute(req).await?;

        let res = ensure(res).await?;
        let update = res.json().await?;
        Ok(update)
    }

    #[allow(unused)]
    pub(crate) async fn delete_release(&self, owner: &str, repo: &str, id: u64) -> Result<()> {
        let mut url = self.url.clone();

        url.path_segments_mut()
            .ok()
            .context("path")?
            .extend(["repos", owner, repo, "releases"])
            .extend([id.to_string()]);

        let req = self.request(Method::DELETE, url)?.build()?;
        let res = self.client.execute(req).await?;

        let res = ensure(res).await?;
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
            .extend(["repos", owner, repo, "releases"]);

        let request = Request {
            tag_name,
            target_commitish,
            name,
            body,
            prerelease,
            draft,
            generate_release_notes: false,
        };

        let req = self.request(Method::POST, url)?.json(&request).build()?;
        let res = self.client.execute(req).await?;

        let res = ensure(res).await?;
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

        url.path_segments_mut()
            .ok()
            .context("path")?
            .extend(["repos", owner, repo, "releases"])
            .extend([id.to_string()]);

        let request = Request {
            tag_name,
            target_commitish,
            name,
            body,
            prerelease,
            draft,
            generate_release_notes: false,
        };

        let req = self.request(Method::PATCH, url)?.json(&request).build()?;
        let res = self.client.execute(req).await?;

        let res = ensure(res).await?;
        let update = res.json().await?;
        Ok(update)
    }

    /// Upload a release asset.
    pub(crate) async fn upload_release_asset<I>(
        &self,
        owner: &str,
        repo: &str,
        release_id: u64,
        name: &str,
        input: I,
        len: u64,
    ) -> Result<Asset>
    where
        I: 'static + AsyncRead + Send + Sync,
    {
        let mut url = self.uploads_url.clone();

        url.path_segments_mut()
            .ok()
            .context("path")?
            .extend(["repos", owner, repo, "releases"])
            .extend([release_id.to_string()])
            .extend(["assets"]);

        url.query_pairs_mut().append_pair("name", name);

        let body = reqwest::Body::wrap_stream(stream_body(input));

        let req = self
            .request(Method::POST, url.clone())?
            .header(header::CONTENT_TYPE, &OCTET_STREAM)
            .header(header::CONTENT_LENGTH, len)
            .body(body)
            .build()?;

        let res = self.client.execute(req).await?;

        let res = ensure(res).await?;
        Ok(res.json().await?)
    }

    /// Delete a release asset.
    pub(crate) async fn delete_release_asset(
        &self,
        owner: &str,
        repo: &str,
        id: u64,
    ) -> Result<()> {
        let mut url = self.url.clone();

        url.path_segments_mut()
            .ok()
            .context("path")?
            .extend(["repos", owner, repo, "releases", "assets"])
            .extend([id.to_string()]);

        let req = self.request(Method::DELETE, url)?.build()?;
        let res = self.client.execute(req).await?;

        let _ = ensure(res).await?;
        Ok(())
    }

    /// Fetch the first page of results.
    pub(crate) async fn get<T>(&self, url: &Url) -> Result<Option<T>>
    where
        T: DeserializeOwned,
    {
        let req = self.request(Method::GET, url.clone())?.build()?;
        let res = self.client.execute(req).await?;

        if res.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }

        let res = ensure(res).await?;
        Ok(Some(res.json().await?))
    }

    /// Fetch the next page of results.
    pub(crate) async fn next_page<T>(&self, paged: &mut T) -> Result<Option<Vec<T::Item>>>
    where
        T: ?Sized + Paginate,
    {
        if let Some(page) = paged.cached_page() {
            return Ok(Some(T::to_items(page)));
        }

        let mut url = paged.url().clone();
        let page = paged.next_page();
        url.query_pairs_mut().append_pair("page", &page.to_string());

        let req = self.request(Method::GET, url.clone())?.build()?;

        let res = self.client.execute(req).await?;

        let res = ensure(res).await?;
        let page = T::to_items(res.json().await?);

        if page.is_empty() {
            return Ok(None);
        }

        Ok(Some(page))
    }

    fn request(&self, method: Method, url: Url) -> Result<reqwest::RequestBuilder> {
        let mut builder = self
            .client
            .request(method, url.clone())
            .header(header::ACCEPT, &ACCEPT);

        match &self.auth {
            Auth::Bearer(token) => {
                builder = builder.bearer_auth(token.as_secret());
            }
            Auth::Basic(auth) => {
                let mut value =
                    header::HeaderValue::try_from(format!("Basic {}", auth.as_secret()))?;
                value.set_sensitive(true);
                builder = builder.header(header::AUTHORIZATION, value);
            }
            Auth::None => {}
        }

        Ok(builder)
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[allow(unused)]
pub(crate) struct Workflows {
    total_count: usize,
    pub(crate) workflows: Vec<Workflow>,
}

#[derive(Debug, Serialize, Deserialize)]
#[allow(unused)]
pub(crate) struct Workflow {
    pub(crate) id: u64,
    node_id: String,
    pub(crate) name: String,
    path: String,
    state: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    url: String,
    html_url: String,
    badge_url: String,
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
    pub(crate) target_commitish: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Object {
    pub(crate) sha: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Reference {
    pub(crate) object: Object,
}

#[derive(Debug, Deserialize)]
pub(crate) struct WorkflowRuns {
    workflow_runs: Vec<WorkflowRun>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct WorkflowRun {
    pub(crate) status: String,
    #[serde(default)]
    pub(crate) conclusion: Option<String>,
    pub(crate) head_branch: String,
    pub(crate) head_sha: String,
    pub(crate) updated_at: DateTime<Utc>,
    #[serde(default)]
    pub(crate) jobs_url: Option<Url>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Job {
    pub(crate) name: String,
    pub(crate) status: String,
    #[serde(default)]
    pub(crate) conclusion: Option<String>,
    pub(crate) started_at: Option<DateTime<Utc>>,
    pub(crate) completed_at: Option<DateTime<Utc>>,
    pub(crate) html_url: Url,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Jobs {
    pub(crate) jobs: Vec<Job>,
}

pub(crate) trait Paginate {
    /// The container of the item being paged.
    type Container: DeserializeOwned;

    /// The item being paged.
    type Item;

    /// Get the URL for the next page to request.
    fn url(&self) -> &Url;

    /// Get the initial page of responses.
    fn cached_page(&mut self) -> Option<Self::Container>;

    /// Advance the page count.
    fn next_page(&mut self) -> usize;

    /// Coerce the container into its interior items.
    fn to_items(container: Self::Container) -> Vec<Self::Item>;
}

pub(crate) struct PagedWorkflowRuns {
    url: Url,
    page: usize,
    first_page: Option<WorkflowRuns>,
}

impl PagedWorkflowRuns {
    fn new(url: Url, first_page: Option<WorkflowRuns>) -> Self {
        Self {
            url,
            page: 0,
            first_page,
        }
    }
}

impl Paginate for PagedWorkflowRuns {
    type Container = WorkflowRuns;
    type Item = WorkflowRun;

    #[inline]
    fn url(&self) -> &Url {
        &self.url
    }

    #[inline]
    fn cached_page(&mut self) -> Option<Self::Container> {
        self.first_page.take()
    }

    #[inline]
    fn next_page(&mut self) -> usize {
        let page = self.page + 1;
        self.page = page;
        page
    }

    #[inline]
    fn to_items(container: Self::Container) -> Vec<Self::Item> {
        container.workflow_runs
    }
}

#[derive(Deserialize)]
pub(crate) struct Paged<T> {
    url: Url,
    page: usize,
    initial: Option<Vec<T>>,
    _marker: PhantomData<T>,
}

impl<T> Paged<T> {
    fn new(url: Url, initial: Vec<T>) -> Self {
        Self {
            url,
            page: 1,
            initial: Some(initial),
            _marker: PhantomData,
        }
    }
}

impl<T> Paginate for Paged<T>
where
    T: DeserializeOwned,
{
    type Container = Vec<T>;
    type Item = T;

    #[inline]
    fn url(&self) -> &Url {
        &self.url
    }

    #[inline]
    fn cached_page(&mut self) -> Option<Self::Container> {
        self.initial.take()
    }

    #[inline]
    fn next_page(&mut self) -> usize {
        let page = self.page + 1;
        self.page = page;
        page
    }

    #[inline]
    fn to_items(container: Self::Container) -> Vec<Self::Item> {
        container
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
