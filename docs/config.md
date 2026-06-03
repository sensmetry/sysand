# Configuration

Sysand offers three ways of configuring its behavior:

- command line options
- environment variables
- configuration files

Command line options override corresponding environment variables, while both
take priority over (but not always override) options in configuration files.

## Configuration files

Sysand will look for a `sysand.toml` file in the current directory or in the
current project directory (see [print-root](commands/root.md) command) if such
has been found. Automatic discovery of configuration files can be disabled with
the command line option `--no-config` or setting the environment variable
`SYSAND_NO_CONFIG` to `true`.

The command line option `--config-file` (or environment variable `SYSAND_CONFIG_FILE`)
can be used to specify the path to an additional config file.

## Configurable options

- [Indexes](config/indexes.md)
- [Dependencies](config/dependencies.md)
