# Authentication

Project indices and remotely stored project kpars (or sources) may require authentication in order
to get authorised access. Sysand currently supports this for:

- HTTP(S) using the [basic access authentication scheme](https://en.wikipedia.org/wiki/Basic_access_authentication)

Support is planned for:

- HTTP(S) with digest access, (fixed) bearer token, and OAuth2 device authentication
- Git with private-key and basic access authentication

## Configuring

At the time of writing authentication can only be configured through environment variables.
Providing credentials is done by setting environment variables following the pattern

```
SYSAND_CRED_<X> = <PATTERN>
SYSAND_CRED_<X>_BASIC_USER = <USER>
SYSAND_CRED_<X>_BASIC_PASS = <PASSWORD>
```

Where `<X>` is arbitrary, `<PATTERN>` is a wildcard (glob) pattern matching URLs, and 
`<USER>:<PASSWORD>` are credentials that may be used with URLs matching the pattern.

Thus, for example,

```
SYSAND_CRED_TEST = "https://*.example.com/**"
SYSAND_CRED_TEST_BASIC_USER = "foo"
SYSAND_CRED_TEST_BASIC_PASS = "bar"
```

Would tell Sysand that it *may* use the credentials `foo:bar` with URLs such as

```
https://www.example.com/projects/project.kpar
https://projects.example.com/entries.txt
https://projects.example.com/projects/myproject/versions.txt
```

In the wildcard pattern, `?` matches any single letter, `*` matches any sequence of characters
not containing `/`, and `**` matches any sequence of characters possibly including `/`.