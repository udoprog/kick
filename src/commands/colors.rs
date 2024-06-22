use termcolor::{Color, ColorSpec};

/// System colors.
pub(super) struct Colors {
    pub(super) skip_cond: ColorSpec,
    pub(super) title: ColorSpec,
    pub(super) matrix: ColorSpec,
    pub(super) warn: ColorSpec,
}

impl Colors {
    /// Construct colors system.
    pub(super) fn new() -> Self {
        let mut skip_cond = ColorSpec::new();
        skip_cond.set_fg(Some(Color::Red));
        skip_cond.set_bold(true);

        let mut title = ColorSpec::new();
        title.set_fg(Some(Color::White));
        title.set_bold(true);

        let mut matrix = ColorSpec::new();
        matrix.set_fg(Some(Color::Yellow));

        let mut warn = ColorSpec::new();
        warn.set_fg(Some(Color::Yellow));

        Self {
            skip_cond,
            title,
            matrix,
            warn,
        }
    }
}
