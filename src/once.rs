use std::{cell::UnsafeCell, ops::Deref};

use anyhow::Result;

/// A value which is calculated once.
///
/// Note that since this is `!Sync`, we can be sure that `get` is only
/// accessible by one thread at a time. Therefore the initialization routine is
/// correctly only run once.
pub(crate) struct Once<T, F> {
    value: UnsafeCell<Option<T>>,
    init: UnsafeCell<Option<F>>,
}

impl<T, F> Once<T, F> {
    /// Construct a new value which is initialized once.
    pub(crate) fn new(init: F) -> Self {
        Self {
            value: UnsafeCell::new(None),
            init: UnsafeCell::new(Some(init)),
        }
    }
}

impl<T, F> Once<T, F>
where
    F: Fn() -> T,
    T: Copy,
{
    /// Get and calculate the interior value.
    pub(crate) fn get(&self) -> T {
        // SAFETY: We have exclusive access to the interior value.
        unsafe {
            if let Some(init) = (*self.init.get()).take() {
                let value = init();
                self.value.get().replace(Some(value));
            }

            (*self.value.get()).unwrap_unchecked()
        }
    }
}

impl<T, F> Once<T, F>
where
    F: Fn() -> Result<T>,
    T: Deref,
{
    /// Get and calculate the interior value.
    pub(crate) fn try_get(&self) -> Result<&T::Target> {
        // SAFETY: We have exclusive access to the interior value.
        unsafe {
            if let Some(init) = (*self.init.get()).take() {
                let value = init()?;
                self.value.get().replace(Some(value));
            }

            Ok((*self.value.get()).as_ref().unwrap_unchecked())
        }
    }
}
