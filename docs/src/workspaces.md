# Workspaces

> [!warning]
> Workspace support is experimental and may change or be removed in any release.
> See [workspaces tracking issue](https://github.com/sensmetry/sysand/issues/101)
> for planned functionality and changes

## Introduction

It is common to have multiple related projects that are all developed together
and that usually use each other's functionality.

Sysand supports such uses with workspaces. A workspace is a collection of
projects, commonly structured like this:

```plain
workspace
 ├──project_1
 ├──project_2
 └──project_3
```

Such a structure is not a requirement, though. Projects can be anywhere
under the workspace root directory.

## Defining a workspace

A workspace is defined by `.workspace.json` file in the root directory of
the workspace.

`.workspace.json` contains a JSON object with the following keys:

- `projects`: An array of objects having two keys:
  - `path`: A Unix-style path relative to workspace root, specifying the
    project's directory
  - `iris`: An array of IRIs identifying the project. The IRIs can be freely
    chosen, but reasonable care has to be taken to make them not clash with
    possible IRIs of third party projects. Any of the included IRIs can be
    used to refer to the project from other projects in the workspace
    instead of using `file://` URLs
- `meta` (optional): An object containing workspace-level metadata:
  - `metamodel` (optional): An IRI specifying a default metamodel that can
    be referenced from project `.meta.json` files using
    `{ "preset": "default" }`. See [Inheriting fields from workspace defaults](#inheriting-fields-from-workspace-defaults).
- `project` (optional): An object with default values for inheritable
  project fields. See [Inheriting fields from workspace defaults](#inheriting-fields-from-workspace-defaults).
- `presets` (optional): A map of named presets, each with their own
  `project` and/or `meta` defaults. See [Inheriting fields from workspace defaults](#inheriting-fields-from-workspace-defaults).

## Example

An example `.workspace.json` file:

```json
{
  "projects": [
    {
      "path": "projectGroup1/project1",
      "iris": ["urn:local:project1"]
    },
    {
      "path": "projectGroup1/project2",
      "iris": ["urn:local:project2"]
    },
    {
      "path": "project3",
      "iris": ["urn:local:project3"]
    }
  ]
}
```

## Inheriting fields from workspace defaults

When many projects in a workspace share the same version, publisher, license,
or metamodel, you can define these values once in `.workspace.json` and
reference them from each project instead of repeating them.

### Root defaults

Define a `project` object at the top level of `.workspace.json`:

```json
{
  "projects": [...],
  "project": {
    "version": "2.0.0",
    "publisher": "Acme Corp",
    "license": "MIT"
  }
}
```

Reference a root default in `.project.json` using `{ "preset": "default" }`:

```json
{
  "name": "my-project",
  "version": { "preset": "default" },
  "publisher": { "preset": "default" },
  "usage": []
}
```

To inherit the workspace-level `metamodel` in `.meta.json`:

```json
{
  "index": { ... },
  "created": "...",
  "metamodel": { "preset": "default" }
}
```

### Named presets

For workspaces with projects that fall into distinct categories (for example
KerML vs SysML projects), you can define named presets under the `presets` key.
Each preset may have a `project` section (for inheritable `.project.json`
fields) and/or a `meta` section (for the `metamodel` field).

```json
{
  "projects": [...],
  "presets": {
    "kerml": {
      "project": { "version": "1.0.0" },
      "meta": { "metamodel": "https://www.omg.org/spec/KerML/20250201" }
    },
    "sysml": {
      "project": { "version": "2.0.0" },
      "meta": { "metamodel": "https://www.omg.org/spec/SysML/20250201" }
    }
  }
}
```

Reference a named preset in `.project.json`:

```json
{
  "name": "my-kerml-project",
  "version": { "preset": "kerml" },
  "usage": []
}
```

Reference a named preset's `metamodel` in `.meta.json`:

```json
{
  "index": { ... },
  "created": "...",
  "metamodel": { "preset": "kerml" }
}
```

### Inheritable fields

| File            | Field                             |
| --------------- | --------------------------------- |
| `.project.json` | `version`, `publisher`, `license` |
| `.meta.json`    | `metamodel`                       |

### Conflict rules

- A field may be defined either in the root `project` section **or** in a
  preset — not both. For example, if `project.version` is set, no preset may
  also set `project.version`.
- Two sibling presets may both define the same field independently (projects
  choose at most one preset per field).
- The preset name `"default"` is reserved; it refers to the root `project`
  defaults and cannot be used as a named preset key.
