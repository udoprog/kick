use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use relative_path::{RelativePath, RelativePathBuf};
use reqwest::{header, StatusCode};
use tokio::sync::mpsc::Sender;
use url::Url;

use crate::file::File;

/// Fake user agent to hopefully have crates.io and others hate us less.
const USER_AGENT: header::HeaderValue =
    header::HeaderValue::from_static("UdoprogProjectChecker/0.0");
const ACCEPT: header::HeaderValue = header::HeaderValue::from_static("text/html");

/// Url testing errors.
pub(crate) struct UrlError {
    /// The URL tested.
    pub(crate) url: Url,
    /// The statuscode returned.
    pub(crate) status: StatusCode,
    /// The test.
    pub(crate) tests: Vec<Test>,
}

#[derive(Debug, Clone)]
pub(crate) struct Test {
    pub(crate) file: Arc<File>,
    pub(crate) range: Range<usize>,
    pub(crate) path: RelativePathBuf,
    pub(crate) line_offset: usize,
    pub(crate) error: Option<Arc<anyhow::Error>>,
}

/// Urls to test.
#[derive(Default)]
pub struct Urls {
    /// Test cases.
    tests: Vec<Test>,
    /// Urls to test, mapped to their corresponding test case.
    urls: HashMap<Url, Vec<usize>>,
    /// Bad urls.
    bad_urls: HashMap<String, Vec<usize>>,
}

impl Urls {
    /// The number of urls to check.
    pub(crate) fn check_urls(&self) -> usize {
        self.urls.len()
    }

    /// Bad URLs.
    pub(crate) fn bad_urls<'a>(&'a self) -> impl Iterator<Item = (&'a str, &'a Test)> + 'a {
        self.bad_urls.iter().flat_map(|(key, values)| {
            values
                .iter()
                .flat_map(|i| self.tests.get(*i).map(|v| (key.as_str(), v)))
        })
    }

    /// Add a URL to test.
    pub(crate) fn insert(
        &mut self,
        url: Url,
        file: Arc<File>,
        range: Range<usize>,
        path: &RelativePath,
        line_offset: usize,
    ) {
        let index = self.tests.len();

        self.tests.push(Test {
            file,
            range,
            path: path.to_owned(),
            line_offset,
            error: None,
        });

        self.urls.entry(url).or_default().push(index);
    }

    /// Add a URL to test.
    pub(crate) fn insert_bad_url(
        &mut self,
        url: String,
        error: anyhow::Error,
        file: Arc<File>,
        range: Range<usize>,
        path: &RelativePath,
        line_offset: usize,
    ) {
        let index = self.tests.len();
        self.tests.push(Test {
            file,
            range,
            path: path.to_owned(),
            line_offset,
            error: Some(Arc::new(error)),
        });
        self.bad_urls.entry(url).or_default().push(index);
    }

    /// Test URLs.
    pub(crate) async fn check_urls_task(
        &self,
        limit: usize,
        tx: Sender<Result<(Url, StatusCode), UrlError>>,
    ) -> Result<()> {
        let client = reqwest::Client::new();

        let mut it = self.urls.iter();
        let mut futures = unicycle::FuturesUnordered::new();

        for (url, indexes) in (&mut it).take(limit) {
            futures.push(self.make_task(&client, url, indexes));
        }

        while let Some(result) = futures.next().await {
            tx.send(result?)
                .await
                .map_err(|_| anyhow!("failed to send"))?;

            if let Some((url, indexes)) = it.next() {
                futures.push(self.make_task(&client, url, indexes));
            }
        }

        Ok(())
    }

    async fn make_task(
        &self,
        client: &reqwest::Client,
        url: &Url,
        index: &Vec<usize>,
    ) -> Result<Result<(Url, StatusCode), UrlError>> {
        let req = client
            .head(url.clone())
            .header(header::USER_AGENT, USER_AGENT)
            .header(header::ACCEPT, ACCEPT)
            .build()?;

        let status = client.execute(req).await?.status();

        if status.is_success() {
            return Ok(Ok((url.clone(), status)));
        }

        let mut tests = Vec::new();

        for i in index {
            tests.extend(self.tests.get(*i).cloned());
        }

        Ok(Err(UrlError {
            url: url.clone(),
            status,
            tests,
        }))
    }
}
