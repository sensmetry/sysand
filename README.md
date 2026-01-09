# Sysand: a package manager for SysML v2 and KerML

> [!important]
> This is an early preview release, intended for early adopters
> to test, integrate, and give feedback. While we hope to keep the tool in a
> usable state, interfaces are subject to change and usability will likely not
> yet be representative of a stable release.

This repository contains Sysand, a [package
manager](https://en.wikipedia.org/wiki/Package_manager) for SysML v2 and KerML
similar to package managers for programming languages such as Pip for Python,
NPM for JavaScript, Maven for Java, and NuGet for .NET. Sysand is based on a
concept of a model interchange project, a slight generalization of a project
interchange file (`*.kpar`), defined in [KerML clause
10.3](https://www.omg.org/spec/KerML/1.0/PDF#page=432).

Sysand can be used as a standalone tool through its command line interface (CLI)
or be integrated into other tools through one of its APIs (currently, Python and
Java are supported).


## Documentation

Sysand usage documentation is provided in User Guide at
[docs.sysand.org](https://docs.sysand.org/).


## Installation

See [installation section in User Guide](http://docs.sysand.org/getting_started/installation.html)
for various ways to download Sysand.


## Contributing

### Development

Development instructions are provided in [DEVELOPMENT.md](DEVELOPMENT.md).

### Legal

For contributors' guidelines regarding legal matters, please see the
[CONTRIBUTING.md](CONTRIBUTING.md) file.

## Licensing

The implementation is dual-licensed under the MIT and Apache-2.0 licenses,
meaning users may choose to use the code under *either* license. Contributors
agree to provide contributed code under **both** licenses.

Sysand is maintained by [Sensmetry](https://www.sensmetry.com), with
contributions from the community. To see the complete list of contributors,
please see the git history.
