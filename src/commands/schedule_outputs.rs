use std::collections::BTreeMap;
use std::rc::Rc;

use super::Env;

/// Schedule outputs.
#[derive(Clone)]
pub(super) struct ScheduleOutputs {
    pub(super) env: Env,
    pub(super) outputs: Rc<BTreeMap<String, String>>,
}
