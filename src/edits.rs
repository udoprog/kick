use std::fmt::{self, Arguments};

use musli::{Decode, Encode};
use nondestructive::yaml;

use crate::keys::Keys;

/// A collection of document edits.
#[derive(Clone, Default, Encode, Decode)]
pub(crate) struct Edits {
    changes: Vec<Change>,
}

impl Edits {
    /// Insert the given key into a mapping identified by `at`.
    pub(crate) fn insert(
        &mut self,
        at: yaml::Id,
        reason: impl fmt::Display,
        key: String,
        value: Value,
    ) {
        self.changes.push(Change::Insert {
            reason: reason.to_string(),
            key,
            value,
            at,
        });
    }

    /// Set the given value at a value identified by `at`.
    pub(crate) fn set(&mut self, at: yaml::Id, reason: impl fmt::Display, value: impl Into<Value>) {
        self.changes.push(Change::Set {
            reason: reason.to_string(),
            value: value.into(),
            at,
        });
    }

    /// Remove the specified key in mapping `mapping`.
    pub(crate) fn remove_key(&mut self, mapping: yaml::Id, reason: impl fmt::Display, key: String) {
        self.changes.push(Change::RemoveKey {
            reason: reason.to_string(),
            key,
            mapping,
        });
    }

    /// Get an iterator over changes.
    pub(crate) fn changes(&self) -> impl Iterator<Item = &'_ Change> {
        self.changes.iter()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    /// Add an edit using a dynamic value.
    pub(crate) fn edit(&mut self, keys: &mut Keys, actual: yaml::Value<'_>, value: Value) {
        match value {
            Value::String(string) => {
                if !actual.as_str().map_or(false, |actual| actual == string) {
                    self.set(
                        actual.id(),
                        format_args!("{keys}: expected string `{string}`"),
                        string.as_str(),
                    );
                }
            }
            Value::Array(array) => {
                let Some(actual) = actual.as_sequence() else {
                    self.set(actual.id(), format_args!("{keys}: expected array"), array);
                    return;
                };

                self.edit_sequence(keys, actual, array);
            }
            Value::Mapping(mapping) => {
                let Some(actual) = actual.as_mapping() else {
                    self.set(
                        actual.id(),
                        format_args!("{keys}: expected mapping"),
                        mapping,
                    );
                    return;
                };

                self.edit_mapping(keys, actual, mapping);
            }
        }
    }

    /// Add an edit using a dynamic mapping.
    pub(crate) fn edit_mapping(
        &mut self,
        keys: &mut Keys,
        mapping: yaml::Mapping<'_>,
        items: impl IntoIterator<Item = (String, Value)>,
    ) {
        for (key, value) in items {
            keys.field(&key);

            let Some(actual) = mapping.get(&key) else {
                self.insert(
                    mapping.id(),
                    format_args!("{keys}: expected mapping"),
                    key,
                    value,
                );

                continue;
            };

            self.edit(keys, actual, value);
            keys.pop();
        }
    }

    /// Add an edit using a dynamic sequence.
    pub(crate) fn edit_sequence<I>(
        &mut self,
        keys: &mut Keys,
        sequence: yaml::Sequence<'_>,
        array: I,
    ) where
        I: IntoIterator<Item = Value>,
        I::IntoIter: ExactSizeIterator,
    {
        let array = array.into_iter();

        if array.len() != sequence.len() {
            self.set(
                sequence.id(),
                format_args!("{keys}: expected array of length {}", array.len()),
                array.into_iter().collect::<Vec<_>>(),
            );

            return;
        }

        for ((index, value), actual) in array.into_iter().enumerate().zip(sequence) {
            keys.index(index);
            self.edit(keys, actual, value);
            keys.pop();
        }
    }
}

/// A stored document change.
#[derive(Clone, Encode, Decode)]
pub(crate) enum Change {
    /// Insert an entry into a map.
    Insert {
        #[musli(with = musli::serde)]
        at: yaml::Id,
        reason: String,
        key: String,
        value: Value,
    },
    /// Oudated version of an action.
    Set {
        #[musli(with = musli::serde)]
        at: yaml::Id,
        reason: String,
        value: Value,
    },
    /// Change to remove a key.
    RemoveKey {
        #[musli(with = musli::serde)]
        mapping: yaml::Id,
        reason: String,
        key: String,
    },
}

#[derive(Clone, Encode, Decode)]
pub(crate) enum Value {
    String(String),
    Array(Vec<Value>),
    Mapping(Vec<(String, Value)>),
}

impl Value {
    pub(crate) fn replace(&self, doc: &mut yaml::Document, at: yaml::Id) {
        match self {
            Value::String(value) => {
                doc.value_mut(at).set_string(value);
            }
            Value::Array(array) => {
                let mut sequence = doc.value_mut(at).make_sequence();
                sequence.clear();

                let mut ids = Vec::with_capacity(array.len());

                for _ in array {
                    ids.push(sequence.push(yaml::Separator::Auto).as_ref().id());
                }

                for (value, id) in array.iter().zip(ids) {
                    value.replace(doc, id);
                }
            }
            Value::Mapping(mapping) => {
                let mut map = doc.value_mut(at).make_mapping();

                let mut ids = Vec::with_capacity(mapping.len());

                for (key, value) in mapping {
                    let id = map.insert(key.clone(), yaml::Separator::Auto).as_ref().id();
                    ids.push((value, id));
                }

                for (value, id) in ids {
                    value.replace(doc, id);
                }
            }
        }
    }
}

impl From<String> for Value {
    fn from(value: String) -> Self {
        Value::String(value)
    }
}

impl From<&str> for Value {
    fn from(value: &str) -> Self {
        Value::String(value.to_owned())
    }
}

impl From<Arguments<'_>> for Value {
    fn from(value: Arguments<'_>) -> Self {
        Value::String(value.to_string())
    }
}

impl From<Vec<Value>> for Value {
    fn from(value: Vec<Value>) -> Self {
        Value::Array(value)
    }
}

impl From<Vec<(String, Value)>> for Value {
    fn from(value: Vec<(String, Value)>) -> Self {
        Value::Mapping(value)
    }
}
