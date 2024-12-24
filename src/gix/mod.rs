mod progress;

use std::num::NonZeroU32;
use std::str;
use std::sync::atomic::AtomicBool;

use anyhow::{Context, Result};
use bstr::{BStr, BString};
use gix::protocol::handshake::Ref;
use gix::remote::fetch::refmap::Source;
use gix::{ObjectId, Repository};

use self::progress::Logger;

/// Sync the local repo by fetching the specified refspecs.
pub(crate) fn sync(
    repo: &Repository,
    url: &str,
    refspecs: &[BString],
    open: bool,
) -> Result<Vec<(BString, ObjectId)>> {
    let mut remote = repo
        .find_fetch_remote(Some(BStr::new(url)))
        .context("Failed to find or make fetch remote")?;

    if open {
        remote = remote.with_fetch_tags(gix::remote::fetch::Tags::None);
    }

    remote = remote.with_refspecs(refspecs, gix::remote::Direction::Fetch)?;

    let options = gix::remote::ref_map::Options::default();

    let mut progress = Logger::new();

    let shallow = NonZeroU32::new(1)
        .map(gix::remote::fetch::Shallow::DepthAtRemote)
        .unwrap_or(gix::remote::fetch::Shallow::NoChange);

    let should_interrupt = AtomicBool::new(false);

    let connect = remote.connect(gix::remote::Direction::Fetch)?;

    let outcome = connect
        .prepare_fetch(&mut progress, options)?
        .with_shallow(shallow)
        .receive(&mut progress, &should_interrupt)?;

    let mut output = Vec::new();

    for mapping in outcome.ref_map.mappings {
        let Source::Ref(r) = mapping.remote else {
            continue;
        };

        match r {
            Ref::Direct {
                full_ref_name,
                object,
            } => {
                output.push((full_ref_name, object));
            }
            Ref::Peeled {
                full_ref_name,
                object,
                ..
            } => {
                output.push((full_ref_name, object));
            }
            other => {
                tracing::warn!("Unexpected ref: {:?}", other);
            }
        }
    }

    Ok(output)
}
