# Project metadata

Contents of interchange project information (`.project.json`)
and metadata (`.meta.json`) files are specified in [KerML
specification](https://www.omg.org/spec/KerML/1.0). Sysand imposes extra
requirements for some fields, as documented below. These extra requirements are
imposed to achieve better interoperability and aid in machine processing.

## Fields

### `version`

Specification does not impose any constraints on how the version should
be formatted, it only recommends to use [semantic versioning][semver] for
versionsSysand strongly encourages users to use semantic versioning and by
default does not accept non-semantic versions in its commands.

### `versionConstraint` (within a `usage`)

Version constraint must be a [semantic versioning range][svconst]. The usage
resolution version selection mechanism is only implemented for semantic version
constraints and semantic versions of used projects. Sysand will not be ablt to
select a version for usages that use non-semantic versions.

[semver]: https://semver.org/
[svconst]: https://github.com/npm/node-semver/tree/v7.7.3?tab=readme-ov-file#ranges

### `license`

Specification does not have any requirements for license format. Sysand
encourages the use of [SPDX License Expressions][spdx] to specify licenses.
By default Sysand rejects `license` strings that are not valid SPDX License
Expressions. Examples of valid expressions:

- `MIT` - MIT license
- `MIT OR Apache-2.0` - MIT and/or Apache-2.0 license applies, chosen by the
  user
- `LicenseRef-My-License` - a custom license (`LicenseRef-` part is mandatory
  in this case). This should be used when the license used is not in
  [SPDX License list][spdxlist].

See the [specification][spdx] and [license list][spdxlist] for more information.
If a project's license is not given, it is assumed that the project cannot be
used under any license.

As specified by [REUSE project][reuse], license files should be included at
the top level directory of the project inside `LICENSES` subdirectory. License
file name should match the `license` field's value with `.txt` appended, except
when using composite expressions (those containing `OR`, `AND` or `WITH`). In
those cases include all the license/exception files mentioned in the expression.
For the complete set of SPDX license and exception files, see the [SPDX GitHub
repository][llist]. Custom licenses (such as `LicenseRef-MyCustom`) result in
license file name `LicenseRef-MyCustom.txt`.

Each file should have its license specified at the top in a comment. See [REUSE
spec][reuseheader] for details.

[spdx]: https://spdx.github.io/spdx-spec/v3.0.1/annexes/spdx-license-expressions/
[spdxlist]: https://spdx.org/licenses/
[reuse]: https://reuse.software/
[llist]: https://github.com/spdx/license-list-data/tree/main/text
[reuseheader]: https://reuse.software/faq/#step-2
