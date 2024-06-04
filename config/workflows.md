### `workflows.<id>`

Defines configuration for a workflow defined in the `.github/workflows/<id>.yml`
file.

<br>

#### Examples

```toml
[workflows.ci]
template = "data/ci.yml"
name = "CI"
branch = "main"
features = [
    "schedule-random-weekly"
]
```
