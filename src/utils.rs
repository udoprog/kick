use core::fmt;

/// Helper to format command outputs.
pub(crate) struct CommandRepr<'a, S>(&'a [S]);

impl<'a, S> CommandRepr<'a, S> {
    pub(crate) fn new(command: &'a [S]) -> Self {
        Self(command)
    }
}

impl<S> fmt::Display for CommandRepr<'_, S>
where
    S: AsRef<str>,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut it = self.0.iter();
        let last = it.next_back();

        for part in it {
            write!(f, "{} ", part.as_ref())?;
        }

        if let Some(part) = last {
            write!(f, "{}", part.as_ref())?;
        }

        Ok(())
    }
}
