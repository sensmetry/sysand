# Build

## README bundling

By default, `sysand build` looks for a `README.md` file at the project root
and includes it in the `.kpar` archive. This allows package indexes to display
README content on package pages.

If no `README.md` file exists, the build proceeds normally without including one.
