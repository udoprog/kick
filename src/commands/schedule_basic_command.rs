use crate::process::OsArg;

use super::Run;

#[derive(Clone)]
pub(crate) struct ScheduleBasicCommand {
    command: OsArg,
    args: Vec<OsArg>,
}

impl ScheduleBasicCommand {
    pub(super) fn new<C, A>(command: C, args: A) -> Self
    where
        C: Into<OsArg>,
        A: IntoIterator<Item: Into<OsArg>>,
    {
        Self {
            command: command.into(),
            args: args.into_iter().map(Into::into).collect(),
        }
    }

    pub(super) fn build(self) -> Run {
        Run::command(self.command, self.args)
    }
}
