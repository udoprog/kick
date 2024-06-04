Maintain actions.

### `[actions.latest]`

Define the latest version of an action that should be used.

This lints that any actions uses this version. 

<br>

#### Examples

```toml
[actions]
latest = [
    { name = "actions/checkout", version = "v4" }
    { name = "actions/download-artifact", version = "v4" }
    { name = "actions/upload-artifact", version = "v4" }
]
```

A table-like structure is also supported:

```toml
[actions.latest]
"actions/checkout" = "v4"
"actions/download-artifact" = "v4"
"actions/upload-artifact" = "v4"
```

### `[actions.deny]`

Deny the use of a particular action, with a reason stating why it is denied.

<br>

#### Examples

```toml
[actions]
deny = [
    { name = "actions-rs/cargo", reason = "Using `run` is less verbose and faster" }
    { name = "actions-rs/toolchain", reason = "Using `run` is less verbose and faster" }
]
```

A table-like structure is also supported:

```toml
[actions.deny]
"actions-rs/cargo" = "Using `run` is less verbose and faster"
"actions-rs/toolchain" = "Using `run` is less verbose and faster"
```
