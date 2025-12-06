use core::cell::RefCell;
use core::fmt::{self, Write};

#[derive(Default, Clone)]
pub struct Keys {
    parts: RefCell<Vec<Part>>,
}

impl Keys {
    pub(crate) fn field(&self, key: &str) {
        self.parts.borrow_mut().push(Part::Field(key.to_owned()));
    }

    pub(crate) fn index(&self, index: usize) {
        self.parts.borrow_mut().push(Part::Index(index));
    }

    pub(crate) fn pop(&self) {
        self.parts.borrow_mut().pop();
    }
}

impl fmt::Display for Keys {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let parts = self.parts.borrow();

        if parts.is_empty() {
            return write!(f, ".");
        }

        for p in parts.iter() {
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
