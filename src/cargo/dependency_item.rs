use toml_edit::Item;

/// A single declared dependency.
#[repr(transparent)]
pub(crate) struct DependencyItem {
    value: Item,
}

impl DependencyItem {
    pub(crate) fn new(value: &Item) -> &Self {
        // SAFETY: type is repr transparent.
        unsafe { &*(value as *const Item as *const Self) }
    }

    /// Get the package of the dependency.
    pub(crate) fn package(&self) -> Option<&str> {
        self.value.get("package").and_then(Item::as_str)
    }

    /// Get the version requirement of the dependency, which is either the
    /// dependency itself if it is a string or its `version` key.
    pub(crate) fn version(&self) -> Option<&str> {
        if let Some(version) = self.value.as_str() {
            return Some(version);
        }

        self.value.get("version").and_then(Item::as_str)
    }

    /// Test if dependency is optional
    pub(crate) fn is_optional(&self) -> Option<bool> {
        self.value.get("optional").and_then(Item::as_bool)
    }

    /// Test if dependency is a workspace dependency.
    pub(crate) fn is_workspace(&self) -> Option<bool> {
        self.value.get("workspace").and_then(Item::as_bool)
    }
}
