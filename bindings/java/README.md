# Java bindings

The current iteration of bindings provides low-level API that exposes the
Sysand commands as static Java methods. For example, currently there are no
methods to programmatically modify `.project.json` or `.library.json` files,
except in a few specific cases such as adding new dependencies. The proper
high-level API is planned in the future once the internals of the Sysand
reached sufficient maturity. Contributions towards the design of the high-level
API are welcome.

## Design choices

There are currently multiple ways to wrap a Rust library for Java:

1. The oldest and most established way is to use JNI (Java Native Interface) via
   `jni` crate. It requires a lot of manual work, but it works with older
   versions of Java and does not require additional dependencies.
2. `java-bindgen` crate provides a nice abstraction layer on top of JNI, but it
   includes additional dependencies, which might cause problems when importing
   the created library.
3. Since Java 22, there is a foreign function and memory API, which allows to
   call Rust functions from Java without using JNI (see [Project
   Panama](https://openjdk.org/projects/panama/)). Unfortunately, this approach
   is not available on Java 21, which is used in the [Pilot][pilot] implementation.

We have decided to use the first approach because it should be the easiest to
integrate for our end-users. We may want to migrate to the foreign function and
memory API once [Pilot][pilot] updates to Java 22 or newer.

Note: From JDK 22, Java throws a warning when loading a native Java module, and
it will become an error in the future. To fix this, user has to explicitly allow
native modules as described in [JEP 472](https://openjdk.org/jeps/472#Description).
Currently, the warning looks as follows:

   ```text
   WARNING: A restricted method in java.lang.System has been called
   WARNING: java.lang.System::load has been called by com.sensmetry.sysand.NativeLoader in an unnamed module (file:.../sysand-X.Y.Z-SNAPSHOT.jar)
   WARNING: Use --enable-native-access=ALL-UNNAMED to avoid a warning for callers in this module
   WARNING: Restricted methods will be blocked in a future release unless native access is enabled
   ```

[pilot]: https://github.com/Systems-Modeling/SysML-v2-Pilot-Implementation

## Building and testing

Requirements:

- Rust version given in `rust-version` in [Cargo.toml](../../Cargo.toml) or later
- Java 8 or later
- maven
- Python 3 (executable named `python3`)

Build and run tests:

```sh
./scripts/run_tests.sh
```

Only build:

```sh
./scripts/java-builder.py build
```

Only run tests:

```sh
./scripts/java-builder.py test
```
