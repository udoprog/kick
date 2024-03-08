use std::fmt::{self, Write};

#[derive(Default, Clone)]
pub struct Keys {
    parts: Vec<Part>,
}

impl Keys {
    pub(crate) fn field(&mut self, key: &str) {
        self.parts.push(Part::Field(key.to_owned()));
    }

    pub(crate) fn index(&mut self, index: usize) {
        self.parts.push(Part::Index(index));
    }

    pub(crate) fn pop(&mut self) {
        self.parts.pop();
    }
}

impl fmt::Display for Keys {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.parts.is_empty() {
            return write!(f, ".");
        }

        let mut it = self.parts.iter();

        if let Some(p) = it.next() {
            write!(f, "{p}")?;
        }

        for p in it {
            if let Part::Field(..) = p {
                f.write_char('.')?;
            }

            write!(f, "{p}")?;
        }

        Ok(())
    }
}

#[derive(Clone)]
enum Part {
    Field(String),
    Index(usize),
}

impl fmt::Display for Part {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Part::Field(key) => {
                write!(f, "{key}")
            }
            Part::Index(index) => {
                write!(f, "[{index}]")
            }
        }
    }
}
