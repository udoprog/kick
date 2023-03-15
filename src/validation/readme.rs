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
use crate::model::{Module, ModuleParams};
use crate::urls::Urls;
use crate::validation::Validation;
use crate::workspace::Package;

/// Name of README to generate.
pub(crate) const README_MD: &str = "README.md";

/// Marker that is put into the generated header to indicate when it ends.
const HEADER_MARKER: &str = "<!--- header -->";

struct Readme<'a, 'outer> {
    module: &'a Module,
    readme_path: &'a RelativePath,
    entry: &'a RelativePath,
    params: ModuleParams<'a>,
    validation: &'outer mut Vec<Validation>,
    urls: &'outer mut Urls,
}

/// Perform readme validation.
pub(crate) fn build(
    cx: &Ctxt<'_>,
    manifest_dir: &RelativePath,
    module: &Module,
    package: &Package,
    params: ModuleParams<'_>,
    validation: &mut Vec<Validation>,
    urls: &mut Urls,
) -> Result<()> {
    let readme_path = manifest_dir.join(README_MD);

    let entry = 'entry: {
        for entry in package.entries() {
            if entry.to_path(cx.root).is_file() {
                break 'entry entry;
            }
        }

        bail!("{manifest_dir}: missing existing entrypoint")
    };

    let mut readme = Readme {
        module,
        readme_path: &readme_path,
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
    if !rm.readme_path.to_path(cx.root).is_file() {
        rm.validation.push(Validation::MissingReadme {
            path: rm.readme_path.to_owned(),
        });
    }

    if rm.entry.to_path(cx.root).is_file() {
        let mut badges = Vec::new();

        for badge in cx.config.badges(&rm.module.path) {
            badges.push(BadgeParams {
                markdown: badge.markdown(rm.params)?,
                html: badge.html(rm.params)?,
            });
        }

        let (file, full, rest) = process_lib_rs(cx, rm, &badges)?;
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

        let readme_from_lib_rs = readme_from_lib_rs(cx, rm, &full, &rest, &badges)?;

        if *file != *full {
            rm.validation.push(Validation::MismatchedLibRs {
                path: rm.entry.to_owned(),
                new_file: full,
            });
        }

        let readme = match File::read(rm.readme_path.to_path(cx.root)) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => File::new(),
            Err(e) => return Err(e.into()),
        };

        if readme != readme_from_lib_rs {
            rm.validation.push(Validation::BadReadme {
                path: rm.readme_path.to_owned(),
                new_file: Arc::new(readme_from_lib_rs),
            });
        }
    }

    Ok(())
}

/// Test if line is a badge comment.
fn is_badge_comment(c: &str) -> bool {
    let c = c.trim();

    if c == "<div align=\"center\">" || c == "</div>" {
        return true;
    }

    if c.starts_with("[<img ") && c.ends_with(")") {
        return true;
    }

    if c.starts_with("[![") && c.ends_with(")") {
        return true;
    }

    if c.starts_with("<a href") && c.ends_with("</a>") {
        return true;
    }

    false
}

#[derive(Serialize)]
struct BadgeParams {
    html: Option<String>,
    markdown: Option<String>,
}

#[derive(Serialize)]
struct TemplateParams<'a> {
    badges: &'a [BadgeParams],
    rest: Option<&'a str>,
    header_marker: Option<&'a str>,
    #[serde(flatten)]
    params: ModuleParams<'a>,
}

#[derive(Serialize)]
struct ReadmeParams<'a> {
    full: Option<&'a str>,
    rest: Option<&'a str>,
    badges: &'a [BadgeParams],
    #[serde(flatten)]
    params: ModuleParams<'a>,
}

/// Process the lib rs.
fn process_lib_rs(
    cx: &Ctxt<'_>,
    rm: &Readme<'_, '_>,
    badges: &[BadgeParams],
) -> Result<(Arc<File>, Arc<File>, File)> {
    let source = File::read(rm.entry.to_path(cx.root))?;
    let mut new_file = File::new();

    let mut source_lines = source.lines().peekable();
    let mut header_marker = None;

    let rest = if let Some(lib) = cx.config.lib(&rm.module.path) {
        while let Some(line) = source_lines.peek().and_then(|line| line.as_rust_comment()) {
            if line.starts_with("#") {
                break;
            }

            if line == HEADER_MARKER {
                header_marker = Some(HEADER_MARKER);
                source_lines.next();
                break;
            }

            source_lines.next();
        }

        let raw: File = source_lines.collect();
        let rest: File = raw
            .lines()
            .flat_map(|line| line.as_rust_comment())
            .collect();

        let lib = lib.render(&TemplateParams {
            badges,
            params: rm.params,
            header_marker,
            rest: rest.as_non_empty_str(),
        })?;

        for string in lib.trim().lines() {
            if string.is_empty() {
                new_file.line("//!");
            } else {
                new_file.line(format_args!("//! {string}"));
            }
        }

        for line in raw.lines() {
            if line.as_rust_comment().is_some() {
                continue;
            }

            new_file.line(line);
        }

        rest
    } else {
        let mut rest = File::new();

        while let Some(line) = source_lines.peek().and_then(|line| line.as_rust_comment()) {
            if !is_badge_comment(line) {
                break;
            }

            source_lines.next();
        }

        for badge in badges {
            if let Some(markdown) = &badge.markdown {
                new_file.line(format_args!("//! {markdown}"));
            }
        }

        for line in source_lines {
            new_file.line(line.as_ref().trim_end());

            if let Some(line) = line.as_rust_comment() {
                rest.line(line.trim_end());
            }
        }

        rest
    };

    new_file.ensure_trailing_newline();
    Ok((Arc::new(source), Arc::new(new_file), rest))
}

/// Test if the specified file has toplevel headings.
fn markdown_checks(readme: &mut Readme<'_, '_>, file: &Arc<File>) -> Result<MarkdownChecks> {
    let mut comment = File::new();

    let mut initial = true;
    let mut checks = MarkdownChecks::default();

    for (offset, line) in file.lines().enumerate() {
        if initial {
            checks.line_offset = offset + 1;
        }

        if let Some(line) = line.as_rust_comment() {
            comment.line(line);
            initial = false;
        }
    }

    let file = Arc::new(comment.clone());

    let opts = Options::empty();

    let parser = Parser::new_with_broken_link_callback(comment.as_str(), opts, None);
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
fn readme_from_lib_rs(
    cx: &Ctxt<'_>,
    rm: &mut Readme<'_, '_>,
    full: &File,
    rest: &File,
    badges: &[BadgeParams],
) -> Result<File> {
    let mut body = File::new();

    let mut in_code_block = None::<bool>;

    for line in full.lines() {
        let comment = match line.as_rust_comment() {
            Some(comment) => comment,
            None => {
                continue;
            }
        };

        if in_code_block == Some(true) && comment.trim_start().starts_with("# ") {
            continue;
        }

        if comment.starts_with("```") {
            if in_code_block.is_none() {
                let (parts, specs) = filter_code_block(comment);
                body.line(format_args!("```{parts}"));
                in_code_block = Some(specs.contains("rust"));
                continue;
            }

            in_code_block = None;
        }

        body.line(comment);
    }

    let mut readme = if let Some(readme) = cx.config.readme(&rm.module.path) {
        let output = readme.render(&ReadmeParams {
            full: full.as_non_empty_str(),
            rest: rest.as_non_empty_str(),
            badges: &badges[..],
            params: rm.params,
        })?;

        let mut readme = File::new();

        for line in output.trim().lines() {
            readme.line(line);
        }

        readme
    } else {
        let mut readme = File::new();
        readme.line(format!("# {name}", name = rm.params.crate_name()));
        readme.line("");

        for line in body.lines() {
            readme.line(line);
        }

        readme
    };

    readme.ensure_trailing_newline();
    Ok(readme)
}

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
