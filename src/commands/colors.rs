use termcolor::{Color, ColorSpec};

/// System colors.
pub(crate) struct Colors {
    pub(crate) skip_cond: ColorSpec,
    pub(crate) title: ColorSpec,
    pub(crate) matrix: ColorSpec,
    pub(crate) warn: ColorSpec,
    pub(crate) red: ColorSpec,
    pub(crate) green: ColorSpec,
    pub(crate) dim: ColorSpec,
}

impl Colors {
    /// Construct colors system.
    pub(crate) fn new() -> Self {
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

        let mut red = ColorSpec::new();
        red.set_fg(Some(Color::Red));

        let mut green = ColorSpec::new();
        green.set_fg(Some(Color::Green));

        let dim = ColorSpec::new();

        Self {
            skip_cond,
            title,
            matrix,
            warn,
            red,
            green,
            dim,
        }
    }
}
