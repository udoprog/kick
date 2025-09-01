use core::fmt;
use std::collections::HashMap;

use anyhow::{Context, Result, bail};

enum Part<'a> {
    Literal(&'a str),
    Variable(&'a str),
}

/// A variable that can be used in a template.
pub(crate) enum Variable<'a> {
    Str(&'a str),
    Display(&'a dyn fmt::Display),
}

impl fmt::Display for Variable<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Variable::Str(s) => f.write_str(s),
            Variable::Display(v) => write!(f, "{v}"),
        }
    }
}

/// A parsed template string that can be rendered with variables.
pub(crate) struct Template<'a> {
    parts: Vec<Part<'a>>,
}

impl<'a> Template<'a> {
    /// Parse a template of `{part}` separated by literal components.
    pub(crate) fn parse(input: &'a str) -> Result<Self> {
        let mut parts = Vec::new();
        let mut remaining = input;

        while let Some(open) = remaining.find('{') {
            // Add literal part before the '{'
            if open > 0 {
                parts.push(Part::Literal(remaining.get(..open).unwrap_or_default()));
            }

            // Advance past the '{'
            remaining = remaining.get(open..).unwrap_or_default();

            if remaining.starts_with("{{") {
                // Handle escaped '{{'
                parts.push(Part::Literal("{"));
                remaining = remaining.get(2..).unwrap_or_default();
                continue;
            }

            remaining = remaining.get(1..).unwrap_or_default();

            // Find closing brace
            let Some(close) = remaining.find('}') else {
                bail!(
                    "Unclosed variable at position {}",
                    input.len() - remaining.len()
                );
            };

            // Extract variable name
            let name = remaining.get(..close).unwrap_or_default().trim();

            if name.is_empty() {
                bail!(
                    "Empty variable name at position {}",
                    input.len() - remaining.len() - 1
                );
            }

            parts.push(Part::Variable(name));

            // Advance past the closing brace
            remaining = remaining.get(close + 1..).unwrap_or_default();
        }

        // Add remaining literal part if any
        if !remaining.is_empty() {
            parts.push(Part::Literal(remaining));
        }

        Ok(Self { parts })
    }

    /// Render a template string with the provided variables.
    pub(crate) fn render(&self, variables: &HashMap<&str, Variable<'_>>) -> Result<String> {
        use std::fmt::Write;

        let mut s = String::new();

        for part in &self.parts {
            match part {
                Part::Literal(value) => {
                    s.push_str(value);
                }
                Part::Variable(var) => {
                    let Some(value) = variables.get(var) else {
                        bail!("No such variable `{var}`");
                    };

                    write!(s, "{value}").context("Rendering template")?;
                }
            }
        }

        Ok(s)
    }
}

#[test]
fn template() {
    let template = Template::parse("{project}-{release}-{arch}-{os}").unwrap();
    let mut variables = HashMap::new();
    variables.insert("project", Variable::Str("my_project"));
    variables.insert("release", Variable::Str("1.0.0"));
    variables.insert("arch", Variable::Str("x86_64"));
    variables.insert("os", Variable::Str("linux"));

    let rendered = template.render(&variables).unwrap();
    assert_eq!(rendered, "my_project-1.0.0-x86_64-linux");
}
