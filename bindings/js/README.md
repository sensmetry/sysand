# WASM bindings

## Setup

Requirements:

- Rust version given in `rust-version` in [Cargo.toml](../../Cargo.toml) or later
- npm
- [wasm-pack](https://github.com/drager/wasm-pack) (can be installed with e. g.
  `cargo install wasm-pack`)

### VS Code

If using VS Code (or other compatible editor like e.g. Codium or Cursor) then
base support for JavaScript should come without any need for extensions. We use
ESlint and Prettier for linting and formatting respectively, and both come with
official extensions [ESLint](https://marketplace.visualstudio.com/items?itemName=dbaeumer.vscode-eslint)
and [Prettier](https://marketplace.visualstudio.com/items?itemName=esbenp.prettier-vscode).

## Building and running tests

Build:

```sh
wasm-pack build
```

Build and run "native" tests using Firefox browser:

```sh
WASM_BINDGEN_USE_BROWSER=1 wasm-pack test --headless --firefox
```

Note for Apple Silicon Macs: wasm-pack does [not yet support Firefox on
arm64](https://github.com/drager/wasm-pack/issues/1449). However, it works fine
if you have Rosetta 2.

See [wasm-bindgen
documentation](https://wasm-bindgen.github.io/wasm-bindgen/wasm-bindgen-test/browsers.html)
for how to run in other browsers, in Node, etc. It is also worth looking into
[`.github/workflows/js-wasm.yml`](https://github.com/sensmetry/sysand/blob/main/.github/workflows/js-wasm.yml)
to see which versions of the tools are tested in CI and, therefore, expected to
work.

To run the Jasmine tests, install the dependencies and run `test:browser` target:

```sh
npm install
npm run test:browser
```

## Formatting and linting

Format and lint all code based on configuration in `.pre-commit-config.yaml`,
either with prek or pre-commit, available to install via uv or pip.

```sh
prek run -a

# like this, you ensure this formatting is run before git commits are made
prek install
```
