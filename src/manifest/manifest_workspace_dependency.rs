use toml_edit::Item;

/// A single declared dependency.
pub(crate) struct ManifestWorkspaceDependency<'a> {
    value: &'a Item,
}

impl<'a> ManifestWorkspaceDependency<'a> {
    pub(crate) fn new(value: &'a Item) -> Self {
        Self { value }
    }

    /// Get the package of the dependency.
    pub(crate) fn package(&self) -> Option<&'a str> {
        self.value.get("package").and_then(Item::as_str)
    }

    /// Test if dependency is optional
    pub(crate) fn is_optional(&self) -> Option<bool> {
        self.value.get("optional").and_then(Item::as_bool)
    }
}
