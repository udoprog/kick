Packaging configuration.

<br>

### General

The following sections describe general types being used in seval places.

<br>

#### Version Requirement

A version requirement is either the sole `*` which means *any version*, or a
string with the format `<operator> <version>` where `<operator>` is one of the
following:

* `<` which specifies that a dependency should use a version lower than the
  specified version.
* `<=` which specifies that a dependency should use a version lower than or
  equal to the specified version.
* `=` which specifies that a dependency should use a version equal to the
  specified version.
* `>=` which specifies that a dependency should use a version equal or greater
  to the specified version.
* `>` which specifies that a dependency should use a version greater than the
  specified version.

The `<version>` field can be anything, and its exact semantic depends on the
package system in use.

<br>

### `[[package.files]]`

Array which defines the files to copy.

An entry in the array supports the following fields:
* `source` which is the source paths being copied. This can also be a wildcard.
* `dest` which is the destination path where the file is being copied. If a
  wildcard is specified as a source, this will always be the directory where the
  files are placed.
* `mode` which is the file mode being applied, by default this uses the existing
  file mode.

<br>

#### Examples

```toml
[[package.files]]
source = "desktop/se.tedro.JapaneseDictionary.desktop"
dest = "usr/share/applications/"

[[package.files]]
source = "gnome/jpv@tedro.se/*"
dest = "usr/share/gnome-shell/extensions/jpv@tedro.se/"
mode = "0775"
```

<br>

### `[[package.rpm.requires]]`

Define an RPM dependency.

Each element has the following fields:
* `package` which is the name of the package being dependend on.
* `version` which is the [version requirement](#version-requirement) of the
  specified package.

<br>

#### Examples

```toml
[package.rpm]
requires = [
    { package = "tesseract-langpack-jpn" }
]
```

A table-like structure is also supported:

```toml
[package.rpm.requires]
"tesseract-langpack-jpn" = "*"
```

### `[package.deb.depends]`

Define an Debian dependency.

Each element has the following fields:
* `package` which is the name of the package being dependend on.
* `version` which is the [version requirement](#version-requirement) of the
  specified package..

<br>

#### Examples

```toml
[package.deb]
depends = [
    { package = "tesseract-ocr-jpn" }
]
```

A table-like structure is also supported:

```toml
[package.deb.depends]
"tesseract-ocr-jpn" = "*"
```

<br>

### Full Example

```toml
[[package.files]]
source = "desktop/se.tedro.JapaneseDictionary.desktop"
dest = "usr/share/applications/"

[[package.files]]
source = "desktop/se.tedro.JapaneseDictionary.png"
dest = "usr/share/icons/hicolor/256x256/apps/"

[[package.files]]
source = "desktop/se.tedro.JapaneseDictionary.service"
dest = "usr/share/dbus-1/services/"

[[package.files]]
source = "desktop/se.tedro.japanese-dictionary.plugins.gschema.xml"
dest = "usr/share/glib-2.0/schemas/"

[[package.files]]
source = "gnome/jpv@tedro.se/*"
dest = "usr/share/gnome-shell/extensions/jpv@tedro.se/"

[package.rpm]
requires = [
    { package = "tesseract-langpack-jpn" }
]

[package.deb]
depends = [
    { package = "tesseract-ocr-jpn" }
]
```
