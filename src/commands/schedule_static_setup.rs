use super::Run;

use crate::rstr::RString;

#[derive(Clone)]
pub(super) struct ScheduleStaticSetup {
    pub(super) command: &'static str,
    pub(super) args: Vec<RString>,
    pub(super) name: &'static str,
    pub(super) skipped: Option<String>,
}

impl ScheduleStaticSetup {
    pub(super) fn build(self) -> Run {
        Run::command(self.command, self.args)
            .with_name(Some(self.name))
            .with_skipped(self.skipped)
    }
}
