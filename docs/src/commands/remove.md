# `sysand remove`

Remove usage from project information

## Usage

```sh
sysand remove [OPTIONS] <IRI>
```

## Description

Removes all instances of IRI from list usages in the project information file
`.project.json`. By default this will also update the lockfile and sync the local
environment (creating one if not already present).

## Arguments

- `<IRI>`: IRI identifying the project usage to be removed
