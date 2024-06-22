use std::fmt;

use anyhow::{ensure, Result};
use termcolor::WriteColor;

use crate::process::Command;

use super::BatchConfig;

/// Suggestions that might arise from a preparation.
#[derive(Default)]
pub(crate) struct Remediations {
    remediations: Vec<Remediation>,
}

impl Remediations {
    /// Test if suggestions are empty.
    pub(crate) fn is_empty(&self) -> bool {
        self.remediations.is_empty()
    }

    pub(super) fn command(&mut self, title: impl fmt::Display, command: Command) {
        self.remediations.push(Remediation::Command {
            title: title.to_string(),
            command,
        });
    }

    /// Apply remediations.
    pub(crate) fn apply<O>(self, o: &mut O, c: &BatchConfig<'_, '_>) -> Result<()>
    where
        O: ?Sized + WriteColor,
    {
        for remediation in self.remediations {
            match remediation {
                Remediation::Command { mut command, .. } => {
                    o.set_color(&c.colors.title)?;
                    writeln!(o, "Running: {}", command.display_with(c.shell))?;
                    o.reset()?;
                    let status = command.status()?;
                    ensure!(status.success(), status);
                }
            }
        }

        Ok(())
    }

    /// Print suggestions.
    pub(crate) fn print<O>(&self, o: &mut O, c: &BatchConfig<'_, '_>) -> Result<()>
    where
        O: ?Sized + WriteColor,
    {
        for remediation in &self.remediations {
            match remediation {
                Remediation::Command { title, command } => {
                    o.set_color(&c.colors.warn)?;
                    writeln!(o, "{title}")?;
                    o.reset()?;

                    writeln!(o, "  run: {}", command.display_with(c.shell))?;
                }
            }
        }

        Ok(())
    }
}

enum Remediation {
    Command { title: String, command: Command },
}
