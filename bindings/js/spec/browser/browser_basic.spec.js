// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//NON-working
//import * as sysand from "sysand";
//const sysand = await import("sysand");;

// // For loading in Node
// let sysand;
// if(require) {
//     sysand = require("../pkg_nodejs_dev/sysand.js");
// }

// For loading in browser
let sysand;
beforeAll(async function () {
  if (!sysand) {
    sysand = await import("sysand");
  }
  sysand.init_logger();
  sysand.ensure_debug_hook();
});

beforeEach(async function () {
  sysand.clear_local_storage("sysand_storage/");
});

it("can initialise a project in browser local storage", async function () {
  // `init` writes a `model.toml`, here as everywhere else. The two
  // json documents are what a *build* produces and what a legacy
  // project has; `init` writes neither.
  //
  // `acme`/`basic-init` rather than `a`/`basic_init`: a publisher and
  // name must normalize to a valid package-URL part, which rules out a
  // single character and an underscore.
  sysand.do_init_js_local_storage(
    "basic-init",
    "acme",
    "1.2.3",
    "sysand_storage",
    "/",
    "MIT OR Apache-2.0",
  );
  expect(window.localStorage.getItem("sysand_storage/model.toml")).toBe(
    'manifest-version = "1.0.0"\n\n' +
      '[project]\n' +
      'publisher = "acme"\n' +
      'name = "basic-init"\n' +
      'version = "1.2.3"\n' +
      "sources = []\n" +
      'license = "MIT OR Apache-2.0"\n',
  );
  expect(window.localStorage.getItem("sysand_storage/.project.json")).toBe(
    null,
  );
  expect(window.localStorage.getItem("sysand_storage/.meta.json")).toBe(null);
});

it("can initialise an empty environment in browser local storage", async function () {
  sysand.do_env_js_local_storage("sysand_storage", "/");
  expect(window.localStorage.key(0)).toBe("sysand_storage/.sysand/entries.txt");
  expect(window.localStorage.key(1)).toBe(null);
  expect(
    window.localStorage.getItem("sysand_storage/.sysand/entries.txt"),
  ).toBe("");
});
