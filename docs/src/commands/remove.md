# `sysand remove`

Remove usage from project information

Will also remove project source overrides from configuration file if available.

## Usage

```sh
sysand remove [OPTIONS] <IRI|PUBLISHER/NAME>
```

## Description

Removes all instances of IRI from list usages in the project information file
`.project.json`. By default this will also update the lockfile and sync the local
environment (creating one if not already present).

## Arguments

- `<IRI|PUBLISHER/NAME>`: IRI identifying the project usage to be removed, or
  `<publisher>/<name>` shorthand for `pkg:sysand/<publisher>/<name>`

{{#include ./partials/global_opts.md}}
