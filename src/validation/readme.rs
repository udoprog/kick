use std::collections::BTreeSet;
use std::ops::Range;
use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use pulldown_cmark::{Event, HeadingLevel, LinkType, Options, Parser, Tag};
use relative_path::RelativePath;
use reqwest::Url;
use serde::Serialize;

use crate::ctxt::Ctxt;
use crate::file::File;
use crate::model::CrateParams;
use crate::urls::Urls;
use crate::validation::Validation;
use crate::workspace::Package;

/// Name of README to generate.
pub(crate) const README_MD: &str = "README.md";

/// Marker that is put into the generated header to indicate when it ends.
const HEADER_MARKER: &str = "<!--- header -->";

struct Readme<'a, 'outer> {
    name: &'a str,
    path: &'a RelativePath,
    entry: &'a RelativePath,
    params: CrateParams<'a>,
    validation: &'outer mut Vec<Validation>,
    urls: &'outer mut Urls,
}

/// Perform readme validation.
pub(crate) fn build(
    cx: &Ctxt<'_>,
    path: &RelativePath,
    name: &str,
    package: &Package,
    params: CrateParams<'_>,
    validation: &mut Vec<Validation>,
    urls: &mut Urls,
) -> Result<()> {
    let readme_path = path.join(README_MD);

    let entry = 'entry: {
        for entry in package.entries() {
            if entry.to_path(cx.root).is_file() {
                break 'entry entry;
            }
        }

        bail!("{name}: missing existing entrypoint")
    };

    let mut readme = Readme {
        name,
        path: &readme_path,
        entry: &entry,
        params,
        validation,
        urls,
    };

    validate(cx, &mut readme).with_context(|| anyhow!("{readme_path}: readme validation"))?;
    Ok(())
}

#[derive(Default)]
struct MarkdownChecks {
    line_offset: usize,
    toplevel_headings: Vec<(Arc<File>, Range<usize>)>,
    missing_preceeding_br: Vec<(Arc<File>, Range<usize>)>,
}

/// Validate the current model.
fn validate(cx: &Ctxt<'_>, rm: &mut Readme<'_, '_>) -> Result<()> {
    if !rm.path.to_path(cx.root).is_file() {
        rm.validation.push(Validation::MissingReadme {
            path: rm.path.to_owned(),
        });
    }

    if rm.entry.to_path(cx.root).is_file() {
        let (file, new_file) = process_lib_rs(cx, rm)?;
        let checks = markdown_checks(rm, &file)?;

        for (file, range) in checks.toplevel_headings {
            rm.validation.push(Validation::ToplevelHeadings {
                path: rm.entry.to_owned(),
                file,
                range,
                line_offset: checks.line_offset,
            });
        }

        for (file, range) in checks.missing_preceeding_br {
            rm.validation.push(Validation::MissingPreceedingBr {
                path: rm.entry.to_owned(),
                file,
                range,
                line_offset: checks.line_offset,
            });
        }

        let readme_from_lib_rs = readme_from_lib_rs(&new_file, rm.params)?;

        if *file != *new_file {
            rm.validation.push(Validation::MismatchedLibRs {
                path: rm.entry.to_owned(),
                new_file: new_file.clone(),
            });
        }

        let readme = match File::read(rm.path.to_path(cx.root)) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => File::new(),
            Err(e) => return Err(e.into()),
        };

        if readme != readme_from_lib_rs {
            rm.validation.push(Validation::BadReadme {
                path: rm.path.to_owned(),
                new_file: Arc::new(readme_from_lib_rs),
            });
        }
    }

    Ok(())
}

/// Process the lib rs.
fn process_lib_rs(
    cx: &Ctxt<'_>,
    readme: &Readme<'_, '_>,
) -> Result<(Arc<File>, Arc<File>), anyhow::Error> {
    /// Test if line is a badge comment.
    fn is_badge_comment(c: &[u8]) -> bool {
        let c = trim_ascii(c);

        if c == b"<div align=\"center\">" || c == b"</div>" {
            return true;
        }

        if c.starts_with(b"[<img ") && c.ends_with(b")") {
            return true;
        }

        if c.starts_with(b"[![") && c.ends_with(b")") {
            return true;
        }

        if c.starts_with(b"<a href") && c.ends_with(b"</a>") {
            return true;
        }

        false
    }

    pub const fn trim_ascii(bytes: &[u8]) -> &[u8] {
        trim_ascii_end(trim_ascii_start(bytes))
    }

    pub const fn trim_ascii_start(mut bytes: &[u8]) -> &[u8] {
        while let [first, rest @ ..] = bytes {
            if first.is_ascii_whitespace() {
                bytes = rest;
            } else {
                break;
            }
        }

        bytes
    }

    pub const fn trim_ascii_end(mut bytes: &[u8]) -> &[u8] {
        while let [rest @ .., last] = bytes {
            if last.is_ascii_whitespace() {
                bytes = rest;
            } else {
                break;
            }
        }

        bytes
    }

    #[derive(Serialize)]
    struct BadgeParams {
        html: Option<String>,
        markdown: Option<String>,
    }

    #[derive(Serialize)]
    struct HeaderParams<'a> {
        badges: &'a [BadgeParams],
        description: Option<&'a str>,
        is_more: bool,
    }

    let source = File::read(readme.entry.to_path(cx.root))?;
    let mut new_file = File::new();

    let mut badges = Vec::new();

    for badge in cx.config.badges(readme.name) {
        badges.push(BadgeParams {
            markdown: badge.markdown(cx, &readme.params)?,
            html: badge.html(cx, &readme.params)?,
        });
    }

    let mut source_lines = source.lines().peekable();

    if let Some(header) = cx.config.header(readme.name) {
        let mut found_marker = false;

        while let Some(line) = source_lines.peek().and_then(|line| line.as_rust_comment()) {
            let line = trim_ascii_start(line);

            if line.starts_with(b"#") {
                break;
            }

            if line == HEADER_MARKER.as_bytes() {
                found_marker = true;
                source_lines.next();
                break;
            }

            source_lines.next();
        }

        let header = header.render(&HeaderParams {
            badges: &badges,
            description: readme.params.description.map(str::trim),
            is_more: source_lines.peek().is_some(),
        })?;

        for string in header.split('\n') {
            if string.is_empty() {
                new_file.push(b"//!");
            } else {
                new_file.push(format!("//! {string}").as_bytes());
            }
        }

        // Add a header marker in case an existing marker was found and
        // there is nothing more in the header.
        if found_marker
            && source_lines
                .peek()
                .and_then(|line| line.as_rust_comment())
                .is_some()
        {
            new_file.push(format!("//! {HEADER_MARKER}").as_bytes());
        }
    } else {
        while let Some(line) = source_lines.peek().and_then(|line| line.as_rust_comment()) {
            if !is_badge_comment(line) {
                break;
            }

            source_lines.next();
        }

        for badge in badges {
            if let Some(markdown) = &badge.markdown {
                new_file.push(format!("//! {markdown}").as_bytes());
            }
        }
    }

    for line in source_lines {
        let bytes = line.as_bytes();
        let bytes = trim_ascii_end(bytes);
        new_file.push(bytes);
    }

    Ok((Arc::new(source), Arc::new(new_file)))
}

/// Test if the specified file has toplevel headings.
fn markdown_checks(readme: &mut Readme<'_, '_>, file: &Arc<File>) -> Result<MarkdownChecks> {
    let mut comment = Vec::new();

    let mut initial = true;
    let mut checks = MarkdownChecks::default();

    for (offset, line) in file.lines().enumerate() {
        if initial {
            checks.line_offset = offset + 1;
        }

        if let Some(line) = line.as_rust_comment() {
            comment.push(std::str::from_utf8(line)?);
            initial = false;
        }
    }

    let comment = comment.join("\n");
    let file = Arc::new(File::from_vec(comment.as_bytes().to_vec()));

    let opts = Options::empty();

    let parser = Parser::new_with_broken_link_callback(&comment, opts, None);
    let mut preceeding_newline = false;

    for (event, range) in parser.into_offset_iter() {
        match event {
            Event::Html(html) => {
                if html.trim() == "<br>" {
                    preceeding_newline = true;
                    continue;
                }
            }
            Event::Start(tag) => match tag {
                Tag::Heading(level, _, _) => {
                    if !preceeding_newline {
                        checks
                            .missing_preceeding_br
                            .push((file.clone(), range.clone()));
                    }

                    if matches!(level, HeadingLevel::H1) {
                        checks.toplevel_headings.push((file.clone(), range.clone()));
                    }
                }
                Tag::Link(LinkType::Autolink, href, _) => {
                    visit_url(readme, href.as_ref(), &file, &range, &checks)?;
                }
                Tag::Link(LinkType::Inline, href, _) => {
                    visit_url(readme, href.as_ref(), &file, &range, &checks)?;
                }
                Tag::Link(LinkType::Shortcut, href, _) => {
                    visit_url(readme, href.as_ref(), &file, &range, &checks)?;
                }
                _ => {}
            },
            _ => {}
        }

        preceeding_newline = false;
    }

    Ok(checks)
}

/// Insert an URL.
fn visit_url(
    readme: &mut Readme<'_, '_>,
    url: &str,
    file: &Arc<File>,
    range: &Range<usize>,
    checks: &MarkdownChecks,
) -> Result<()> {
    // Link to anchor does nothing.
    if url.starts_with('#') {
        return Ok(());
    }

    let error = match str::parse::<Url>(url) {
        Ok(url) if matches!(url.scheme(), "http" | "https") => {
            readme.urls.insert(
                url,
                file.clone(),
                range.clone(),
                readme.entry,
                checks.line_offset,
            );

            return Ok(());
        }
        Ok(url) => anyhow!("only 'http://' or 'https://' urls are supported, got `{url}`"),
        Err(e) => e.into(),
    };

    readme.urls.insert_bad_url(
        url.to_owned(),
        error,
        file.clone(),
        range.clone(),
        readme.entry,
        checks.line_offset,
    );

    Ok(())
}

/// Generate a readme.
fn readme_from_lib_rs(file: &File, params: CrateParams<'_>) -> Result<File> {
    /// Filter code block fragments.
    fn filter_code_block(comment: &str) -> (String, BTreeSet<String>) {
        let parts = comment.get(3..).unwrap_or_default();
        let mut out = BTreeSet::new();

        for part in parts.split(',') {
            let part = part.trim();

            match part {
                "" => continue,
                "no_run" => continue,
                "should_panic" => continue,
                "ignore" => continue,
                "edition2018" => continue,
                "edition2021" => continue,
                _ => {}
            }

            out.insert(part.to_owned());
        }

        if out.is_empty() {
            out.insert(String::from("rust"));
        }

        (out.iter().cloned().collect::<Vec<_>>().join(","), out)
    }

    let mut readme = File::new();

    let mut in_code_block = None::<bool>;
    let name = params.name;

    readme.push(format!("# {name}").as_bytes());
    readme.push(b"");

    for line in file.lines() {
        let comment = match line.as_rust_comment() {
            Some(comment) => std::str::from_utf8(comment)?,
            None => {
                continue;
            }
        };

        let comment = if let Some(" ") = comment.get(..1) {
            comment.get(1..).unwrap_or_default()
        } else {
            comment
        };

        if in_code_block == Some(true) && comment.trim_start().starts_with("# ") {
            continue;
        }

        if comment.starts_with("```") {
            if in_code_block.is_none() {
                let (parts, specs) = filter_code_block(comment);
                readme.push(format!("```{parts}").as_bytes());
                in_code_block = Some(specs.contains("rust"));
                continue;
            }

            in_code_block = None;
        }

        readme.push(comment.as_bytes());
    }

    readme.ensure_trailing_newline();
    Ok(readme)
}
