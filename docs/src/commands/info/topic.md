# `sysand info topic`

Get or manipulate the list of topics of the project

## Usage

```sh
sysand info topic [OPTIONS]
```

## Description

Prints the list of topics of the given project. With modifying options, updates the list.

## Options

- `--numbered`: Prints a numbered list
- `--set <TOPIC>`: Replace the entire list with a single topic
- `--add <TOPIC>`: Append a topic to the list
- `--remove <N>`: Remove the topic at position N (1-based, as shown by `--numbered`)
- `--clear`: Remove all topics

{{#include ../partials/global_opts.md}}
