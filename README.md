# trunk github action

Github action that installs and runs a [`trunk`] command.

[`trunk`]: https://github.com/thedodd/trunk

## Inputs

### `args`

**Optional** Arguments to pass to `trunk`. Defaults to `build`.

### `version`

**Optional** The version of `trunk` to use. Must match a tagged release. Defaults to `latest`.

See: https://github.com/thedodd/trunk

### `wasm-bindgen-version`

**Optional** The version of `wasm-bindgen` to install. Must match a tagged release. Defaults to `latest`.

See: https://github.com/rustwasm/wasm-bindgen

### `binaryen-version`

**Optional** The version of `binaryen` to install. Must match a tagged release. Defaults to `latest`.

See: https://github.com/WebAssembly/binaryen

## Example usage

```yaml
- uses: udoprog/trunk-action@v1
  with:
    wasm-bindgen-version: latest
    version: latest
    args: build
```
