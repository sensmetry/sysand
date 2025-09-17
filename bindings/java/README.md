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
   is not available on Java 21, which is used in the Pilot implementation.

We have decided to use the first approach because it should be the easiest to
integrate for our end-users.
