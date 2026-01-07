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

`.workspace.json` contains a JSON object. The only currently permitted key
is `projects`, for which the value is an array of objects having two keys:

- `path`: A Unix-style path relative to workspace root, specifying the
  project's directory
- `iris`: An array of IRIs identifying the project. The IRIs can be freely
  chosen, but reasonable care has to be taken to make them not clash with
  possible IRIs of third party projects. Any of the included IRIs can be
  used to refer to the project from other projects in the workspace
  instead of using `file://` URLs

## Example

An example `.workspace.json` file:

```json
{
    "projects": [
        {
            "path": "projectGroup1/project1",
            "iris": [
                "urn:local:project1"
            ]
        },
        {
            "path": "projectGroup1/project2",
            "iris": [
                "urn:local:project2"
            ]
        },
        {
            "path": "project3",
            "iris": [
                "urn:local:project3"
            ]
        }
    ]
}
```
