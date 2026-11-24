use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use relative_path::RelativePathBuf;

use crate::RepoOptions;
use crate::ctxt::Ctxt;
use crate::repo_sets::{SetKind, validate_name};

#[derive(Debug, Parser)]
pub(crate) struct Opts {
    #[command(subcommand)]
    pub(crate) action: Action,
}

impl Opts {
    pub(crate) fn repo(&self) -> Option<&RepoOptions> {
        match &self.action {
            Action::Create { repo, .. }
            | Action::Add { repo, .. }
            | Action::Remove { repo, .. } => Some(repo),
            Action::Ls
            | Action::Show { .. }
            | Action::Delete { .. }
            | Action::Rename { .. }
            | Action::Edit { .. } => None,
        }
    }

    pub(crate) fn is_edit(&self) -> bool {
        matches!(self.action, Action::Edit { .. })
    }
}

#[derive(Debug, Subcommand)]
pub(crate) enum Action {
    /// List all available persistent and temporary sets.
    #[command(alias = "list")]
    Ls,
    /// Print the repositories in a set.
    Show {
        /// Name of the set to show.
        name: String,
    },
    /// Create a persistent set from the selected repositories.
    Create {
        /// Name of the persistent set to create.
        name: String,
        /// Overwrite an existing persistent set.
        #[arg(long, short = 'f')]
        force: bool,
        #[command(flatten)]
        repo: RepoOptions,
    },
    /// Add the selected repositories to a persistent set.
    Add {
        /// Name of the persistent set to update.
        name: String,
        /// Create the persistent set if it does not exist yet.
        #[arg(long)]
        create: bool,
        #[command(flatten)]
        repo: RepoOptions,
    },
    /// Remove the selected repositories from a persistent set.
    Remove {
        /// Name of the persistent set to update.
        name: String,
        #[command(flatten)]
        repo: RepoOptions,
    },
    /// Delete a persistent or temporary set.
    #[command(alias = "rm")]
    Delete {
        /// Name of the set to delete.
        name: String,
    },
    /// Rename a persistent set.
    #[command(alias = "mv")]
    Rename {
        /// Existing persistent set name.
        old: String,
        /// New persistent set name.
        new: String,
    },
    /// Open a persistent set file in `$VISUAL` or `$EDITOR`.
    Edit {
        /// Name of the persistent set to edit.
        name: String,
    },
}

pub(crate) fn entry(cx: &mut Ctxt<'_>, opts: &Opts) -> Result<()> {
    match &opts.action {
        Action::Ls => {
            let summaries = cx.sets.list()?;

            if summaries.is_empty() {
                println!("No sets found");
                return Ok(());
            }

            let name_width = summaries.iter().map(|s| s.name.len()).max().unwrap_or(0);

            for summary in summaries {
                let count = plural(summary.count, "repo", "repos");
                let mut details = Vec::new();

                if summary.kind == SetKind::Temporary {
                    if let Some(latest) = summary.latest {
                        details.push(latest.format("%Y-%m-%d %H:%M:%S").to_string());
                    }

                    if summary.snapshots > 0 {
                        details.push(plural(summary.snapshots, "snapshot", "snapshots"));
                    }
                }

                if summary.shadowed {
                    details.push(String::from("shadowed by persistent set"));
                }

                if details.is_empty() {
                    println!(
                        "{:<name_width$}  {:<10}  {count}",
                        summary.name, summary.kind
                    );
                } else {
                    println!(
                        "{:<name_width$}  {:<10}  {count} ({})",
                        summary.name,
                        summary.kind,
                        details.join(", ")
                    );
                }
            }
        }
        Action::Show { name } => {
            validate_name(name)?;

            let Some((_, set)) = cx.sets.load_with_kind(name)? else {
                bail!("Set `{name}` not found");
            };

            for repo in set.iter() {
                println!("{repo}");
            }
        }
        Action::Create { name, force, .. } => {
            let repos: Vec<RelativePathBuf> = cx.repos().map(|r| r.path().to_owned()).collect();
            let path = cx.sets.create_persistent(name, &repos, *force)?;
            let count = plural(repos.len(), "repo", "repos");
            println!(
                "Created persistent set `{name}` with {count} ({})",
                path.display()
            );
        }
        Action::Add { name, create, .. } => {
            let repos: Vec<RelativePathBuf> = cx.repos().map(|r| r.path().to_owned()).collect();
            let (added, path) = cx.sets.add_persistent(name, &repos, *create)?;
            let count = plural(added, "repo", "repos");
            println!(
                "Added {count} to persistent set `{name}` ({})",
                path.display()
            );
        }
        Action::Remove { name, .. } => {
            let repos: Vec<RelativePathBuf> = cx.repos().map(|r| r.path().to_owned()).collect();
            let (removed, path) = cx.sets.remove_persistent(name, &repos)?;
            let count = plural(removed, "repo", "repos");
            println!(
                "Removed {count} from persistent set `{name}` ({})",
                path.display()
            );
        }
        Action::Delete { name } => {
            let kind = cx.sets.delete(name)?;
            println!("Deleted {kind} set `{name}`");
        }
        Action::Rename { old, new } => {
            let path = cx.sets.rename_persistent(old, new)?;
            println!(
                "Renamed persistent set `{old}` to `{new}` ({})",
                path.display()
            );
        }
        Action::Edit { name } => {
            let path = cx.sets.persistent_path_for_edit(name)?;
            crate::cli::project::run_editor(&path)?;
        }
    }

    Ok(())
}

fn plural(count: usize, singular: &str, plural: &str) -> String {
    if count == 1 {
        format!("{count} {singular}")
    } else {
        format!("{count} {plural}")
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::{Action, Opts};

    #[test]
    fn parse_subcommands_and_aliases() {
        let ls = Opts::try_parse_from(["set", "ls"]).unwrap();
        assert!(matches!(ls.action, Action::Ls));
        assert!(ls.repo().is_none());

        let list = Opts::try_parse_from(["set", "list"]).unwrap();
        assert!(matches!(list.action, Action::Ls));

        let show = Opts::try_parse_from(["set", "show", "core"]).unwrap();
        assert!(matches!(show.action, Action::Show { ref name } if name == "core"));
        assert!(show.repo().is_none());

        let create =
            Opts::try_parse_from(["set", "create", "core", "--force", "--set", "bad"]).unwrap();
        assert!(matches!(
            create.action,
            Action::Create { ref name, force: true, .. } if name == "core"
        ));
        assert!(create.repo().is_some());

        let add =
            Opts::try_parse_from(["set", "add", "core", "--create", "-p", "repos/a"]).unwrap();
        assert!(matches!(
            add.action,
            Action::Add { ref name, create: true, .. } if name == "core"
        ));
        assert!(add.repo().is_some());

        let remove = Opts::try_parse_from(["set", "remove", "core", "-n", "kick"]).unwrap();
        assert!(matches!(remove.action, Action::Remove { ref name, .. } if name == "core"));
        assert!(remove.repo().is_some());

        let del = Opts::try_parse_from(["set", "rm", "bad"]).unwrap();
        assert!(matches!(del.action, Action::Delete { ref name } if name == "bad"));
        assert!(del.repo().is_none());

        let mv = Opts::try_parse_from(["set", "mv", "old", "new"]).unwrap();
        assert!(matches!(
            mv.action,
            Action::Rename { ref old, ref new } if old == "old" && new == "new"
        ));
        assert!(mv.repo().is_none());

        let edit = Opts::try_parse_from(["set", "edit", "core"]).unwrap();
        assert!(edit.is_edit());
        assert!(edit.repo().is_none());
    }
}
