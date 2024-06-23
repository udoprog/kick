use super::Run;

use crate::rstr::RString;

#[derive(Clone)]
pub(super) struct ScheduleStaticSetup {
    command: &'static str,
    name: &'static str,
    args: Vec<RString>,
    skipped: Option<String>,
}

impl ScheduleStaticSetup {
    pub(super) fn new(
        command: &'static str,
        name: &'static str,
        args: Vec<RString>,
        skipped: Option<String>,
    ) -> Self {
        Self {
            command,
            name,
            args,
            skipped,
        }
    }

    pub(super) fn build(self) -> Run {
        Run::command(self.command, self.args)
            .with_name(Some(self.name))
            .with_skipped(self.skipped)
    }
}
