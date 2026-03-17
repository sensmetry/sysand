# Build

The `[build]` section configures the behavior of the `sysand build` command.

## README.md bundling

By default, `sysand build` looks for a `README.md` file at the project root and
includes it in the `.kpar` archive. This allows package indexes to display
readme content on package pages. Regardless of the source filename, the file is
always stored as `README.md` inside the archive.

If no `README.md` file exists the build proceeds normally without including one.

### Requiring a readme

To explicitly require that `README.md` exists (the build will fail if it is
missing):

```toml
[build]
readme = true
```

### Configuring the readme source file

To bundle a different markdown file as `README.md` in the `.kpar` archive:

```toml
[build]
readme = "some/document.md"
```

The path is relative to the project root and must use forward slashes (`/`).
Subdirectory paths like `docs/README.md` are supported, but absolute paths and
`..` traversal are not allowed. The file must have a `.md` extension.

When a path is explicitly configured, the build will fail if the file does not
exist.

### Disabling readme bundling

To explicitly disable readme bundling:

```toml
[build]
readme = false
```
