# WASM bindings

To run "native" tests as wasm install `wasm-pack` (for example `cargo install
wasm-pack`) and run:

```bash
$ WASM_BINDGEN_USE_BROWSER=1 wasm-pack test --headless --firefox
<...>
```

See [wasm-bindgen
documentation](https://wasm-bindgen.github.io/wasm-bindgen/wasm-bindgen-test/browsers.html)
for how to run in other browser, in Node, etc. It is also worth looking into
.github/workflows/js.yml to see which versions of the tools are tested in CI
and, therefore, expected to work.

To run the Jasmine tests, install the dependencies and run `test:browser` target:

```bash
$ npm install
<...>
$ npm run test:browser
<...>
```
