<!-- markdownlint-disable MD041 -->

## Resolution options

- `--index [<INDEX>...]`: Comma-delimited list of index URLs to use when
  resolving project(s) and/or their dependencies, in addition to the default
  indexes
- `--default-index [<DEFAULT_INDEX>...]`: Comma-delimited list of URLs to use
  as default index URLs. Default indexes are tried before other indexes
  (default `https://beta.sysand.org`)
- `--no-index`: Do not use any index when resolving project(s) and/or their
  dependencies
- `--include-std`: Don't ignore KerML/SysML v2 standard libraries if specified
  as dependencies
