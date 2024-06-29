use anyhow::{bail, Result};

use crate::ctxt::Ctxt;
use crate::SharedOptions;

pub(crate) async fn entry(cx: &mut Ctxt<'_>, _: &SharedOptions) -> Result<()> {
    let client = cx.octokit()?;

    let Some(release) = client.latest_release(crate::OWNER, crate::REPO).await? else {
        bail!("No release found");
    };

    _ = release;
    println!("Latest release: {}", release.tag_name);
    Ok(())
}
