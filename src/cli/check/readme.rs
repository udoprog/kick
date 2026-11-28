use std::iter::Peekable;
use std::ops::Range;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use pulldown_cmark::{Event, HeadingLevel, LinkType, Options, Parser, Tag, TagEnd};
use relative_path::RelativePath;
use reqwest::Url;
use serde::Serialize;

use crate::cargo::Manifest;
use crate::changes::{Change, Warning};
use crate::ctxt::Ctxt;
use crate::file::{File, Lines};
use crate::model::{Repo, RepoParams};
use crate::urls::Urls;

mod items;
mod links;

/// Name of README to generate.
pub(crate) const README_MD: &str = "README.md";

/// Marker that is put into the generated header to indicate when it ends.
const HEADER_MARKER: &str = "<!--- header -->";

struct Readme<'a, 'outer> {
    repo: &'a Repo,
    readme_path: &'a RelativePath,
    entry: &'a RelativePath,
    params: &'a RepoParams<'a>,
    urls: &'outer mut Urls,
    docs: links::DocsRs<'a>,
    do_readme: bool,
    do_lib: bool,
    primary: bool,
}

/// Perform readme change.
pub(crate) fn build(
    cx: &Ctxt<'_>,
    manifest_dir: &RelativePath,
    repo: &Repo,
    manifest: &Manifest,
    params: &RepoParams<'_>,
    urls: &mut Urls,
    do_readme: bool,
    do_lib: bool,
    primary: bool,
) -> Result<()> {
    let readme_path = manifest_dir.join(README_MD);

    let entry = 'entry: {
        for entry in manifest.entries() {
            if cx.to_path(&entry).is_file() {
                break 'entry entry;
            }
        }

        bail!("{}: missing existing entrypoint", cx.display(manifest_dir))
    };

    let package = manifest.as_package();
    let lib_name = manifest
        .lib()
        .and_then(|lib| lib.get("name"))
        .and_then(|name| name.as_str());

    let crate_ident = if manifest.lib().is_some() || entry.file_name() == Some("lib.rs") {
        lib_name
            .or_else(|| package.and_then(|p| p.name().ok()))
            .map(|name| name.replace('-', "_"))
    } else {
        None
    };

    let docs = links::DocsRs {
        package: package.and_then(|p| p.name().ok()),
        version: package.and_then(|p| p.version()),
        crate_ident,
        entry: &entry,
    };

    let mut readme = Readme {
        repo,
        readme_path: &readme_path,
        entry: &entry,
        params,
        urls,
        docs,
        do_readme,
        do_lib,
        primary,
    };

    validate(cx, &mut readme).with_context(|| anyhow!("{readme_path}: readme change"))?;
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
    if !cx.is_file(&cx.to_path(rm.readme_path)) {
        cx.warning(Warning::MissingReadme {
            path: rm.readme_path.to_owned(),
        });
    }

    if !cx.is_file(&cx.to_path(rm.entry)) {
        return Ok(());
    }

    let mut lib_badges = Vec::new();

    for badge in cx.config.lib_badges(rm.repo.path()) {
        lib_badges.push(BadgeParams {
            markdown: badge.markdown(rm.params)?,
            html: badge.html(rm.params)?,
        });
    }

    let wordmark = match cx.config.wordmark(rm.repo) {
        Some(wordmark) if rm.primary => Some(WordmarkParams {
            markdown: wordmark.markdown(rm.params)?,
            html: wordmark.html(rm.params)?,
        }),
        _ => None,
    };

    let (file, lib_rs, comments) = process_lib_rs(cx, rm, wordmark.as_ref(), &lib_badges)?;

    if rm.do_lib && *file != *lib_rs {
        cx.change(Change::UpdateLib {
            path: rm.entry.to_owned(),
            lib: lib_rs,
        })?;
    }

    let checks = markdown_checks(rm, &file)?;

    for (file, range) in checks.toplevel_headings {
        cx.warning(Warning::ToplevelHeadings {
            path: rm.entry.to_owned(),
            file,
            range,
            line_offset: checks.line_offset,
        });
    }

    for (file, range) in checks.missing_preceeding_br {
        cx.warning(Warning::MissingPreceedingBr {
            path: rm.entry.to_owned(),
            file,
            range,
            line_offset: checks.line_offset,
        });
    }

    let mut readme_badges = Vec::new();

    for badge in cx.config.readme_badges(rm.repo.path()) {
        readme_badges.push(BadgeParams {
            markdown: badge.markdown(rm.params)?,
            html: badge.html(rm.params)?,
        });
    }

    let comments = rewrite_links(cx, rm, &file, &comments)?;

    let readme_from_lib_rs =
        readme_from_lib_rs(cx, rm, &comments, wordmark.as_ref(), &readme_badges)?;

    let path = cx.to_path(rm.readme_path);

    let readme = match cx.read_path_to_string(&path)? {
        Some(string) => File::from_string(string),
        None => File::new(),
    };

    if rm.do_readme && readme != readme_from_lib_rs {
        cx.change(Change::UpdateReadme {
            path: rm.readme_path.to_owned(),
            readme: Arc::new(readme_from_lib_rs),
        })?;
    }

    Ok(())
}

/// Test if line is a badge comment.
fn is_badge_comment(c: &str) -> bool {
    let c = c.trim();

    if c == "<div align=\"center\">" || c == "</div>" {
        return true;
    }

    if c.starts_with("[<img ") && c.ends_with(')') {
        return true;
    }

    if c.starts_with("[![") && c.ends_with(')') {
        return true;
    }

    if c.starts_with("<a href") && c.ends_with("</a>") {
        return true;
    }

    false
}

/// Test if line is a standalone image, such as a wordmark.
fn is_image_comment(c: &str) -> bool {
    let c = c.trim();
    c.starts_with("<img ") && c.ends_with('>')
}

/// Test if the line is part of the generated header.
fn is_header_comment(c: &str, wordmark: bool) -> bool {
    is_badge_comment(c) || (wordmark && is_image_comment(c))
}

/// Skip over the generated header, including blank lines which separate parts
/// of it such as the wordmark and the badges.
fn skip_header(lines: &mut Peekable<Lines<'_>>, wordmark: bool) {
    loop {
        let mut ahead = lines.clone();

        while ahead
            .peek()
            .and_then(|line| line.as_rust_comment())
            .is_some_and(|line| line.trim().is_empty())
        {
            ahead.next();
        }

        let Some(line) = ahead.peek().and_then(|line| line.as_rust_comment()) else {
            break;
        };

        if !is_header_comment(line, wordmark) {
            break;
        }

        ahead.next();
        *lines = ahead;
    }
}

#[derive(Serialize)]
struct WordmarkParams {
    html: String,
    markdown: String,
}

#[derive(Serialize)]
struct BadgeParams {
    html: Option<String>,
    markdown: Option<String>,
}

#[derive(Serialize)]
struct TemplateParams<'a> {
    wordmark: Option<&'a WordmarkParams>,
    badges: &'a [BadgeParams],
    body: Option<&'a str>,
    header_marker: Option<&'a str>,
    #[serde(flatten)]
    params: &'a RepoParams<'a>,
}

#[derive(Serialize)]
struct ReadmeParams<'a> {
    body: Option<&'a str>,
    wordmark: Option<&'a WordmarkParams>,
    badges: &'a [BadgeParams],
    #[serde(flatten)]
    params: &'a RepoParams<'a>,
}

/// Process the lib rs.
fn process_lib_rs(
    cx: &Ctxt<'_>,
    rm: &Readme<'_, '_>,
    wordmark: Option<&WordmarkParams>,
    badges: &[BadgeParams],
) -> Result<(Arc<File>, Arc<File>, File)> {
    let path = cx.to_path(rm.entry);

    let source = cx
        .read_path_to_string(&path)?
        .with_context(|| anyhow!("{}: missing file", path.display()))?;

    let source = File::from_string(source);
    let mut lib_rs = File::new();

    let mut source_lines = source.lines().peekable();
    let mut header_marker = None;

    let comments = if let Some(lib) = cx.config.lib(rm.repo) {
        while let Some(line) = source_lines.peek().and_then(|line| line.as_rust_comment()) {
            if line.starts_with('#') {
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

        let comments: File = raw
            .lines()
            .flat_map(|line| line.as_rust_comment())
            .collect();

        let lib = lib.render(&TemplateParams {
            wordmark,
            badges,
            params: rm.params,
            header_marker,
            body: comments.as_non_empty_str(),
        })?;

        for string in lib.trim().lines() {
            if string.is_empty() {
                lib_rs.line("//!");
            } else {
                lib_rs.line(format_args!("//! {string}"));
            }
        }

        for line in raw.lines() {
            if line.as_rust_comment().is_some() {
                continue;
            }

            lib_rs.line(line);
        }

        comments
    } else {
        let mut comments = File::new();
        let has_wordmark = wordmark.is_some();

        skip_header(&mut source_lines, has_wordmark);

        if let Some(wordmark) = wordmark {
            lib_rs.line(format_args!("//! {}", wordmark.markdown));

            if badges.iter().any(|b| b.markdown.is_some()) {
                lib_rs.line("//!");
            }
        }

        for badge in badges {
            if let Some(markdown) = &badge.markdown {
                lib_rs.line(format_args!("//! {markdown}"));
            }
        }

        for line in source_lines {
            lib_rs.line(line.as_ref().trim_end());

            if let Some(line) = line.as_rust_comment() {
                comments.line(line.trim_end());
            }
        }

        comments
    };

    lib_rs.ensure_trailing_newline();
    Ok((Arc::new(source), Arc::new(lib_rs), comments))
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

    let parser = Parser::new_ext(comment.as_str(), opts);
    let mut preceeding_newline = false;
    let mut block = 0u32;

    for (event, range) in parser.into_offset_iter() {
        match event {
            Event::Html(html) if html.trim() == "<br>" => {
                preceeding_newline = true;
                continue;
            }
            Event::Start(tag) => match tag {
                Tag::HtmlBlock => {
                    block += 1;
                }
                Tag::Heading { level, .. } => {
                    if !preceeding_newline {
                        checks
                            .missing_preceeding_br
                            .push((file.clone(), range.clone()));
                    }

                    if matches!(level, HeadingLevel::H1) {
                        checks.toplevel_headings.push((file.clone(), range.clone()));
                    }
                }
                Tag::Link {
                    link_type: LinkType::Autolink,
                    dest_url,
                    ..
                } => {
                    visit_url(readme, dest_url.as_ref(), &file, &range, &checks)?;
                }
                Tag::Link {
                    link_type: LinkType::Inline,
                    dest_url,
                    ..
                } => {
                    visit_url(readme, dest_url.as_ref(), &file, &range, &checks)?;
                }
                Tag::Link {
                    link_type: LinkType::Shortcut,
                    dest_url,
                    ..
                } => {
                    visit_url(readme, dest_url.as_ref(), &file, &range, &checks)?;
                }
                _ => {}
            },
            Event::End(TagEnd::HtmlBlock) => {
                block = block.saturating_sub(1);

                if preceeding_newline {
                    continue;
                }
            }
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
    // Links to anchors do nothing, and intra-doc links are rewritten when the
    // README is generated.
    if url.starts_with('#') || links::is_item_link(url) {
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

struct CtxtSources<'a, 'cx>(&'a Ctxt<'cx>);

impl links::Sources for CtxtSources<'_, '_> {
    fn read(&self, path: &RelativePath) -> Result<Option<String>> {
        self.0.read_path_to_string(&self.0.to_path(path))
    }
}

/// Rewrite intra-doc links in the crate documentation to docs.rs.
fn rewrite_links(
    cx: &Ctxt<'_>,
    rm: &Readme<'_, '_>,
    source: &File,
    comments: &File,
) -> Result<File> {
    let errors = match links::rewrite(comments.as_str(), &rm.docs, &CtxtSources(cx)) {
        Ok(rewritten) => return Ok(File::from_string(rewritten)),
        Err(errors) => errors,
    };

    // The documentation comments are the trailing `//!` lines of the source.
    let comment_lines = source
        .lines()
        .enumerate()
        .filter(|(_, line)| line.as_rust_comment().is_some())
        .map(|(index, _)| index)
        .collect::<Vec<_>>();

    let skip = comment_lines.len().saturating_sub(comments.lines().count());

    let mut message = String::from("cannot rewrite intra-doc links for the README:");

    for error in errors {
        let (position, _) = comments.line_column(error.offset)?;

        let line = comment_lines
            .get(skip.saturating_add(position.line))
            .map_or(0, |index| index.saturating_add(1));

        message.push_str(&format!(
            "\n  {}:{line}: `{}`: {}",
            rm.entry, error.link, error.reason
        ));
    }

    bail!(message)
}

/// Generate a readme.
fn readme_from_lib_rs(
    cx: &Ctxt<'_>,
    rm: &mut Readme<'_, '_>,
    comments: &File,
    wordmark: Option<&WordmarkParams>,
    badges: &[BadgeParams],
) -> Result<File> {
    let mut body = File::new();

    let mut in_code_block = None::<(bool, String)>;

    for line in comments.lines() {
        match &in_code_block {
            Some((_, ticks)) if line.as_ref() == ticks => {
                in_code_block = None;
            }
            // Skip over rust comments.
            Some((true, _)) if line.as_ref().trim_start().starts_with("# ") => {
                continue;
            }
            // Detect a code block and store the ticks that it uses.
            None if (line.as_ref().starts_with("```") || line.as_ref().starts_with("~~~")) => {
                let (specs, ticks) = filter_code_block(line.as_ref());
                let is_rust = specs.iter().any(|item| item == "rust");
                let parts = specs.join(",");
                body.line(format_args!("{ticks}{parts}"));
                in_code_block = Some((is_rust, ticks));
                continue;
            }
            _ => {}
        }

        body.line(line);
    }

    let mut readme = if let Some(readme) = cx.config.readme(rm.repo) {
        let output = readme.render(&ReadmeParams {
            body: body.as_non_empty_str(),
            wordmark,
            badges,
            params: rm.params,
        })?;

        let mut readme = File::new();

        for line in output.trim().lines() {
            readme.line(line);
        }

        readme
    } else {
        let mut readme = File::new();
        readme.line(format!("# {name}", name = rm.params.name()));

        if let Some(wordmark) = wordmark {
            readme.line("");
            readme.line(&wordmark.markdown);
        }

        if !badges.is_empty() {
            readme.line("");

            for badge in badges {
                if let Some(markdown) = &badge.markdown {
                    readme.line(format_args!("{markdown}"));
                }
            }

            readme.line("");
        }

        for line in body.lines() {
            readme.line(line);
        }

        readme
    };

    readme.ensure_trailing_newline();
    Ok(readme)
}

/// Filter code block fragments.
fn filter_code_block(comment: &str) -> (Vec<String>, String) {
    let index = comment
        .find(|c| !(c == '`' || c == '~'))
        .unwrap_or(comment.len());

    let ticks = comment.get(..index).unwrap_or_default();
    let parts = comment.get(index..).unwrap_or_default();

    let mut out = Vec::new();

    for part in parts.split(',') {
        let part = part.trim();

        match part {
            "" | "no_run" | "should_panic" | "compile_fail" | "ignore" | "edition2018"
            | "edition2021" | "no_miri" => continue,
            _ => {}
        }

        out.push(part.to_owned());
    }

    if out.is_empty() {
        out.push(String::from("rust"));
    }

    (out, ticks.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rest(source: &str, wordmark: bool) -> String {
        let file = File::from_string(source.to_owned());
        let mut lines = file.lines().peekable();
        skip_header(&mut lines, wordmark);
        lines.map(|l| format!("{l}\n")).collect()
    }

    const BADGE: &str = "//! [<img alt=\"a\" src=\"b\" height=\"20\">](c)";
    const WORDMARK: &str = "//! <img alt=\"w\" src=\"w.svg\" height=\"64\">";

    #[test]
    fn skips_badges() {
        let source = format!("{BADGE}\n{BADGE}\n//!\n//! Body.\n");
        assert_eq!(rest(&source, false), "//!\n//! Body.\n");
    }

    #[test]
    fn keeps_image_without_wordmark() {
        let source = format!("{BADGE}\n{WORDMARK}\n//!\n//! Body.\n");
        assert_eq!(
            rest(&source, false),
            format!("{WORDMARK}\n//!\n//! Body.\n")
        );
    }

    #[test]
    fn skips_wordmark_and_duplicated_badges() {
        let source = format!("{BADGE}\n{WORDMARK}\n//!\n{BADGE}\n//!\n//! Body.\n");
        assert_eq!(rest(&source, true), "//!\n//! Body.\n");
    }

    struct MapSources(&'static [(&'static str, &'static str)]);

    impl links::Sources for MapSources {
        fn read(&self, path: &RelativePath) -> Result<Option<String>> {
            Ok(self
                .0
                .iter()
                .find(|(p, _)| *p == path.as_str())
                .map(|(_, s)| (*s).to_owned()))
        }
    }

    const LIB_RS: &str = "\
pub struct Foo;
pub enum Color { Red }
pub trait Run {
    type Output;
    const LIMIT: usize = 1;
    fn run(&self) -> Self::Output;
    fn provided(&self) {}
    fn Output() {}
}
pub union Bits { a: u32 }
pub const fn make() {}
pub const LIMIT: u32 = 1;
pub static mut STATE: u32 = 0;
pub type Alias = u32;
pub mod a;
pub mod inline {}
pub fn twice() {}
pub mod twice {}

impl Foo {
    pub fn method(&self) {}
}

#[macro_export]
macro_rules! root {}
";

    const A_RS: &str = "\
pub mod b;
pub struct Bar { pub field: u32 }
";

    const B_RS: &str = "\
pub fn deep() {}
";

    const NESTED_RS: &str = "\
#[macro_export]
/// Docs.
macro_rules! nested {}
";

    const SOURCES: MapSources = MapSources(&[
        ("my-crate/src/lib.rs", LIB_RS),
        ("my-crate/src/a.rs", A_RS),
        ("my-crate/src/a/b/mod.rs", B_RS),
        ("my-crate/src/a/b/nested.rs", NESTED_RS),
    ]);

    const B_WITH_NESTED: MapSources = MapSources(&[
        ("my-crate/src/lib.rs", LIB_RS),
        ("my-crate/src/a.rs", A_RS),
        ("my-crate/src/a/b/mod.rs", "mod nested;\n"),
        ("my-crate/src/a/b/nested.rs", NESTED_RS),
    ]);

    const BASE: &str = "https://docs.rs/my-crate/1.2.3/my_crate/";

    fn docs(lib: bool) -> links::DocsRs<'static> {
        links::DocsRs {
            package: Some("my-crate"),
            version: Some("1.2.3"),
            crate_ident: lib.then(|| "my_crate".to_owned()),
            entry: RelativePath::new("my-crate/src/lib.rs"),
        }
    }

    fn rewrite_with(sources: &MapSources, input: &str) -> String {
        match links::rewrite(input, &docs(true), sources) {
            Ok(output) => output,
            Err(errors) => panic!("{input}: {errors:?}"),
        }
    }

    fn rewrite(input: &str) -> String {
        rewrite_with(&SOURCES, input)
    }

    fn rewrite_err(input: &str) -> Vec<String> {
        match links::rewrite(input, &docs(true), &SOURCES) {
            Ok(output) => panic!("{input}: expected error, got {output}"),
            Err(errors) => errors.into_iter().map(|e| e.reason).collect(),
        }
    }

    #[test]
    fn rewrites_item_kinds() {
        for (input, page) in [
            ("[Foo]", "struct.Foo.html"),
            ("[`Foo`]", "struct.Foo.html"),
            ("[`crate::Foo`]", "struct.Foo.html"),
            ("[`self::Foo`]", "struct.Foo.html"),
            ("[`Color`]", "enum.Color.html"),
            ("[`Run`]", "trait.Run.html"),
            ("[`Bits`]", "union.Bits.html"),
            ("[`make`]", "fn.make.html"),
            ("[`make()`]", "fn.make.html"),
            ("[`LIMIT`]", "constant.LIMIT.html"),
            ("[`STATE`]", "static.STATE.html"),
            ("[`Alias`]", "type.Alias.html"),
            ("[`a`]", "a/index.html"),
            ("[`crate`]", "index.html"),
            ("[`root!`]", "macro.root.html"),
            ("[`macro@root`]", "macro.root.html"),
            ("[`struct@Foo`]", "struct.Foo.html"),
            ("[`fn@twice`]", "fn.twice.html"),
            ("[`mod@twice`]", "twice/index.html"),
            ("[`twice()`]", "fn.twice.html"),
        ] {
            let text = input.trim_start_matches('[').trim_end_matches(']');
            assert_eq!(
                rewrite(&format!("See {input}.\n")),
                format!("See [{text}]({BASE}{page}).\n"),
                "{input}"
            );
        }
    }

    #[test]
    fn rewrites_members_and_modules() {
        for (input, page) in [
            ("a::Bar", "a/struct.Bar.html"),
            ("crate::a::b::deep", "a/b/fn.deep.html"),
            ("a::b", "a/b/index.html"),
            ("Foo::method", "struct.Foo.html#method.method"),
            ("Foo::other()", "struct.Foo.html#method.other"),
            ("Color::Red", "enum.Color.html#variant.Red"),
            ("field@a::Bar::field", "a/struct.Bar.html#structfield.field"),
            ("const@Foo::MAX", "struct.Foo.html#associatedconstant.MAX"),
            ("Run::run", "trait.Run.html#tymethod.run"),
            ("Run::run()", "trait.Run.html#tymethod.run"),
            ("method@Run::run", "trait.Run.html#tymethod.run"),
            ("Run::provided", "trait.Run.html#method.provided"),
            ("fn@Run::provided", "trait.Run.html#method.provided"),
            ("Run::LIMIT", "trait.Run.html#associatedconstant.LIMIT"),
            (
                "const@Run::LIMIT",
                "trait.Run.html#associatedconstant.LIMIT",
            ),
            ("type@Run::Output", "trait.Run.html#associatedtype.Output"),
            ("Run::Output()", "trait.Run.html#method.Output"),
        ] {
            assert_eq!(
                rewrite(&format!("[`{input}`]\n")),
                format!("[`{input}`]({BASE}{page})\n"),
                "{input}"
            );
        }
    }

    #[test]
    fn rewrites_link_forms() {
        assert_eq!(
            rewrite("[text](Foo) and [text](<crate::Foo> \"title\")\n"),
            format!("[text]({BASE}struct.Foo.html) and [text]({BASE}struct.Foo.html \"title\")\n")
        );

        assert_eq!(
            rewrite("[text][Foo] and [Foo][]\n"),
            format!("[text]({BASE}struct.Foo.html) and [Foo]({BASE}struct.Foo.html)\n")
        );

        assert_eq!(
            rewrite("[text][foo] and [foo]\n\n[foo]: crate::Foo\n[url]: https://example.com\n"),
            format!(
                "[text][foo] and [foo]\n\n[foo]: {BASE}struct.Foo.html\n[url]: https://example.com\n"
            )
        );

        assert_eq!(
            rewrite_with(&B_WITH_NESTED, "[`nested!`]\n"),
            format!("[`nested!`]({BASE}macro.nested.html)\n")
        );
    }

    #[test]
    fn leaves_other_text_alone() {
        let input = "[x] [some words] [unknown] [#anchor](#anchor) [web](https://example.com)\n\n```rust\n// [`Foo`]\n```\n";
        assert_eq!(rewrite(input), input);
    }

    #[test]
    fn rejects_unresolvable_links() {
        for (input, reason) in [
            ("[`Missing`]", "no matching public item `Missing`"),
            ("[text](Missing)", "no matching public item `Missing`"),
            ("[`std::fmt::Debug`]", "no public module or type `std`"),
            ("[`Vec<T>`]", "generic parameters"),
            ("[`twice`]", "ambiguous"),
            ("[`inline::Item`]", "no matching public item `Item`"),
            (
                "[`Run::missing`]",
                "no matching member `missing` declared in trait `Run`",
            ),
            (
                "[`const@Run::run`]",
                "no matching member `run` declared in trait `Run`",
            ),
            ("[`type@Run::provided`]", "no matching member `provided`"),
            ("[`Run::Output`]", "`Output` is ambiguous"),
            ("[`field@Run::run`]", "unsupported member disambiguator"),
            ("[`Foo::missing`]", "cannot tell what kind of member"),
            ("[`super::Foo`]", "relative paths"),
            ("[`nested!`]", "no `#[macro_export]` macro named `nested`"),
            ("[`prim@str`]", "not supported"),
            ("[guide](config.md)", "relative links"),
            ("[guide]\n\n[guide]: ./config.md", "relative links"),
        ] {
            let errors = rewrite_err(&format!("{input}\n"));
            assert_eq!(errors.len(), 1, "{input}: {errors:?}");
            assert!(errors[0].contains(reason), "{input}: {errors:?}");
        }
    }

    #[test]
    fn rejects_links_without_lib_target() {
        let errors =
            links::rewrite("[`Foo`] [Foo] [unknown]\n", &docs(false), &SOURCES).unwrap_err();
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(
            errors
                .iter()
                .all(|e| e.reason.contains("no library target"))
        );
    }

    const STRUCTURED_LIB_RS: &str = r#"
pub mod outer {
    pub mod inner {
        pub struct Deep;

        #[macro_export]
        macro_rules! inline_macro { () => {} }
    }

    mod private {
        pub struct Hidden;
    }

    pub use self::private::Hidden as Shown;
}

#[path = "elsewhere/custom.rs"]
pub mod custom;

pub use crate::outer::inner::Deep as Renamed;
pub use self::outer::Shown;
pub use std::fmt::Debug;
pub use outer::inner::*;

pub struct Foo { pub field: u32, private: u32, pub both: u32 }
pub struct Other;
pub enum Kind { Lower, r#Upper(u32) }

impl Foo {
    pub fn method(&self) {}
    pub fn both(&self) {}
    pub const LIMIT: u32 = 1;
    fn private_method(&self) {}
}

impl Iterator for Other {
    type Item = u32;
    fn next(&mut self) -> Option<u32> { None }
}

impl Other {
    /// A string with `pub fn method() {}` in it is not a method.
    pub const TEXT: &str = "pub fn method() {}";
}
"#;

    const STRUCTURED: MapSources = MapSources(&[
        ("my-crate/src/lib.rs", STRUCTURED_LIB_RS),
        (
            "my-crate/src/elsewhere/custom.rs",
            "pub fn custom() {}\npub mod nested;\n",
        ),
        ("my-crate/src/elsewhere/nested.rs", "pub struct Nested;\n"),
    ]);

    #[test]
    fn rewrites_structured_items() {
        for (input, page) in [
            ("outer::inner::Deep", "outer/inner/struct.Deep.html"),
            ("outer::Shown", "outer/struct.Shown.html"),
            ("Shown", "outer/struct.Shown.html"),
            ("Renamed", "outer/inner/struct.Deep.html"),
            ("custom::custom", "custom/fn.custom.html"),
            ("custom::nested::Nested", "custom/nested/struct.Nested.html"),
            ("inline_macro!", "macro.inline_macro.html"),
            ("Foo::field", "struct.Foo.html#structfield.field"),
            ("Foo::LIMIT", "struct.Foo.html#associatedconstant.LIMIT"),
            ("Foo::method", "struct.Foo.html#method.method"),
            ("Other::next", "struct.Other.html#method.next"),
            ("Other::Item", "struct.Other.html#associatedtype.Item"),
            ("Kind::Lower", "enum.Kind.html#variant.Lower"),
            ("Kind::Upper", "enum.Kind.html#variant.Upper"),
        ] {
            assert_eq!(
                rewrite_with(&STRUCTURED, &format!("[`{input}`]\n")),
                format!("[`{input}`]({BASE}{page})\n"),
                "{input}"
            );
        }
    }

    #[test]
    fn rejects_unresolvable_structured_links() {
        for (input, reason) in [
            ("[`Other::method`]", "cannot tell what kind of member"),
            ("[`Foo::private`]", "cannot tell what kind of member"),
            ("[`Foo::private_method`]", "cannot tell what kind of member"),
            ("[`Foo::both`]", "`both` is ambiguous"),
            ("[`Hidden`]", "no matching public item `Hidden`"),
            (
                "[`outer::private::Hidden`]",
                "no public module or type `private`",
            ),
            ("[`Debug`]", "no matching public item `Debug`"),
            ("[`Deep`]", "no matching public item `Deep`"),
        ] {
            let errors = match links::rewrite(&format!("{input}\n"), &docs(true), &STRUCTURED) {
                Ok(output) => panic!("{input}: expected error, got {output}"),
                Err(errors) => errors.into_iter().map(|e| e.reason).collect::<Vec<_>>(),
            };

            assert_eq!(errors.len(), 1, "{input}: {errors:?}");
            assert!(errors[0].contains(reason), "{input}: {errors:?}");
        }
    }

    #[test]
    fn stops_at_code() {
        let source = format!("{WORDMARK}\n//!\n{BADGE}\n\nuse std::fmt;\n");
        assert_eq!(rest(&source, true), "\nuse std::fmt;\n");
    }
}
