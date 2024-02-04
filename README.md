# github action to download latest kick command

Github action that installs [`kick`].

[`kick`]: https://github.com/udoprog/kick

## Inputs

### `version`

**Optional** The version of `kick` to use. Must match a tagged release. Defaults to `latest`.

See: https://github.com/udoprog/kick

## Example usage

```yaml
- uses: udoprog/kick@nightly
  with:
    version: latest
```
