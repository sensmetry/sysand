# Indexes

Sysand defaults to using the index [beta.sysand.org](https://beta.sysand.org)
but it is possible to specify additional indexes or override this default index.

URLs for additional indexes can be specified with the command line argument `--index`.
Multiple indexes can be given by invoking the argument multiple times, or by passing
a comma separated list of URLs. The corresponding environment variable is
`SYSAND_INDEX`, which also accepts comma separated lists of URLs.

Indexes specified with command line arguments or environment variables are
prioritized when resolving dependencies but do not override indexes defined in
configuration files.

The command line argument `--default-index` (and environment variable
`SYSAND_DEFAULT_INDEX`) functions the same as `--index` but will override the default
index `https://beta.sysand.org` and any default indexes defined in configuration
files.

## Defining an index in a configuration file

To add an index with the URL `https://index.example.org` simple add the following
entry to your `sysand.toml`:

```toml
[[index]]
url = "https://index.example.org"
```

To override the default index add `default = true`:

```toml
[[index]]
url = "https://index.example.org"
default = true
```

Indexes are prioritized in the order they are given, with default indexes always
having lower priority compared to regular indexes.
