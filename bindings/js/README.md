# WASM bindings

## Building and running tests

Requirements:

- Rust 1.85 or later
- npm
- [wasm-pack](https://github.com/drager/wasm-pack) (can be installed with e. g.
`cargo install wasm-pack`)

Build:
```bash
wasm-pack build
```

Build and run "native" tests using Firefox browser:

```bash
WASM_BINDGEN_USE_BROWSER=1 wasm-pack test --headless --firefox
```

Note for Apple Silicon Macs: wasm-pack does [not yet support Firefox on
arm64](https://github.com/drager/wasm-pack/issues/1449). However, it works fine
if you have Rosetta 2.

See [wasm-bindgen
documentation](https://wasm-bindgen.github.io/wasm-bindgen/wasm-bindgen-test/browsers.html)
for how to run in other browsers, in Node, etc. It is also worth looking into
[`.github/workflows/js.yml`](https://github.com/sensmetry/sysand/blob/main/.github/workflows/js.yml)
to see which versions of the tools are tested in CI and, therefore, expected to work.

To run the Jasmine tests, install the dependencies and run `test:browser` target:

```bash
npm install
npm run test:browser
```
