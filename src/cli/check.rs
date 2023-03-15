use std::io::Write;

use anyhow::Result;
use clap::Parser;

use crate::ctxt::Ctxt;
use crate::file::{File, LineColumn};
use crate::urls::UrlError;
use crate::urls::Urls;
use crate::validation::Validation;

#[derive(Default, Parser)]
pub(crate) struct Opts {
    /// Perform URL checks where we go out and try and fetch every references URL.
    #[arg(long)]
    url_checks: bool,
    /// Filter by the specified modules.
    #[arg(long = "module", short = 'm', name = "module")]
    modules: Vec<String>,
}

/// Entrypoint to run action.
pub(crate) async fn entry(cx: &Ctxt<'_>, opts: &Opts, fix: bool) -> Result<()> {
    let mut validation = Vec::new();
    let mut urls = Urls::default();

    for module in &cx.modules {
        if crate::should_skip(&opts.modules, module) {
            return Ok(());
        }

        crate::validation::build(cx, module, &mut validation, &mut urls)?;
    }

    for validation in &validation {
        validate(cx, validation, fix)?;
    }

    let o = std::io::stdout();
    let mut o = o.lock();

    for (url, test) in urls.bad_urls() {
        let path = &test.path;
        let (line, column, string) =
            temporary_line_fix(&test.file, test.range.start, test.line_offset)?;

        if let Some(error) = &test.error {
            writeln!(o, "{path}:{line}:{column}: bad url: `{url}`: {error}")?;
        } else {
            writeln!(o, "{path}:{line}:{column}: bad url: `{url}`")?;
        }

        writeln!(o, "{string}")?;
    }

    if opts.url_checks {
        url_checks(&mut o, urls).await?;
    }

    Ok(())
}

/// Report and apply a asingle validation.
fn validate(cx: &Ctxt<'_>, error: &Validation, fix: bool) -> Result<()> {
    Ok(match error {
        Validation::MissingWorkflow {
            path,
            candidates,
            crate_params,
        } => {
            println!("{path}: Missing workflow");

            for candidate in candidates.iter() {
                println!("  Candidate: {candidate}");
            }

            if fix {
                if let [from] = candidates.as_ref() {
                    println!("{path}: Rename from {from}",);
                    std::fs::rename(from.to_path(cx.root), path.to_path(cx.root))?;
                } else {
                    let path = path.to_path(cx.root);

                    if let Some(parent) = path.parent() {
                        if !parent.is_dir() {
                            std::fs::create_dir_all(&parent)?;
                        }
                    }

                    let Some(string) = cx.config.default_workflow(cx, crate_params)? else {
                        println!("  Missing default workflow!");
                        return Ok(());
                    };

                    std::fs::write(path, string)?;
                }
            }
        }
        Validation::DeprecatedWorkflow { path } => {
            println!("{path}: Reprecated Workflow");
        }
        Validation::WrongWorkflowName {
            path,
            actual,
            expected,
        } => {
            println!("{path}: Wrong workflow name: {actual} (actual) != {expected} (expected)");
        }
        Validation::OutdatedAction {
            path,
            name,
            actual,
            expected,
        } => {
            println!(
                "{path}: Outdated action `{name}`: {actual} (actual) != {expected} (expected)"
            );
        }
        Validation::DeniedAction { path, name, reason } => {
            println!("{path}: Denied action `{name}`: {reason}");
        }
        Validation::CustomActionsCheck { path, name, reason } => {
            println!("{path}: Action validation failed `{name}`: {reason}");
        }
        Validation::MissingReadme { path } => {
            println!("{path}: Missing README");
        }
        Validation::MismatchedLibRs { path, new_file } => {
            if fix {
                println!("{path}: Fixing lib.rs");
                std::fs::write(path.to_path(cx.root), new_file.as_bytes())?;
            } else {
                println!("{path}: Mismatched lib.rs");
            }
        }
        Validation::BadReadme { path, new_file } => {
            if fix {
                println!("{path}: Fixing README.md");
                std::fs::write(path.to_path(cx.root), new_file.as_bytes())?;
            } else {
                println!("{path}: Bad README.md");
            }
        }
        Validation::ToplevelHeadings {
            path,
            file,
            range,
            line_offset,
        } => {
            let (line, column, string) = temporary_line_fix(&file, range.start, *line_offset)?;
            println!("{path}:{line}:{column}: doc comment has toplevel headings");
            println!("{string}");
        }
        Validation::MissingPreceedingBr {
            path,
            file,
            range,
            line_offset,
        } => {
            let (line, column, string) = temporary_line_fix(&file, range.start, *line_offset)?;
            println!("{path}:{line}:{column}: missing preceeding <br>");
            println!("{string}");
        }
        Validation::MissingFeature { path, feature } => {
            println!("{path}: missing features `{feature}`");
        }
        Validation::NoFeatures { path } => {
            println!("{path}: trying featured build (--all-features, --no-default-features), but no features present");
        }
        Validation::MissingEmptyFeatures { path } => {
            println!("{path}: missing empty features build");
        }
        Validation::MissingAllFeatures { path } => {
            println!("{path}: missing all features build");
        }
        Validation::CargoTomlIssues {
            path,
            cargo: modified_cargo,
            issues,
        } => {
            println!("{path}:");

            for issue in issues {
                println!("  {issue}");
            }

            if fix {
                if let Some(modified_cargo) = modified_cargo {
                    modified_cargo.save_to(path.to_path(cx.root))?;
                }
            }
        }
        Validation::ActionMissingKey {
            path,
            key,
            expected,
            actual,
        } => {
            println!("{path}: {key}: action missing key, expected {expected}");

            match actual {
                Some(value) => {
                    println!("  actual:");
                    serde_yaml::to_writer(std::io::stdout(), value)?;
                }
                None => {
                    println!("  actual: *missing value*");
                }
            }
        }
        Validation::ActionOnMissingBranch { path, key, branch } => {
            println!("{path}: {key}: action missing branch `{branch}`");
        }
        Validation::ActionExpectedEmptyMapping { path, key } => {
            println!("{path}: {key}: action expected empty mapping");
        }
    })
}

/// Perform url checks.
async fn url_checks<O>(o: &mut O, urls: Urls) -> Result<()>
where
    O: Write,
{
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);

    let total = urls.check_urls();
    let checks = urls.check_urls_task(3, tx);
    tokio::pin!(checks);
    let mut count = 1;
    let mut completed = false;

    loop {
        tokio::select! {
            result = checks.as_mut(), if !completed => {
                result?;
                completed = true;
            }
            result = rx.recv() => {
                let result = match result {
                    Some(result) => result,
                    None => break,
                };

                match result {
                    Ok(_) => {}
                    Err(UrlError { url, status, tests }) => {
                        writeln!(o, "{count:>3}/{total} {url}: {status}")?;

                        for test in tests {
                            let path = &test.path;
                            let (line, column, string) = temporary_line_fix(&test.file, test.range.start, test.line_offset)?;
                            writeln!(o, "  {path}:{line}:{column}: {string}")?;
                        }
                    }
                }

                count += 1;
            }
        }
    }

    Ok(())
}

/// Temporary line comment fix which adjusts the line and column.
fn temporary_line_fix(file: &File, pos: usize, line_offset: usize) -> Result<(usize, usize, &str)> {
    let (LineColumn { line, column }, string) = file.line_column(pos)?;
    let line = line_offset + line;
    let column = column + 4;
    Ok((line, column, string))
}
