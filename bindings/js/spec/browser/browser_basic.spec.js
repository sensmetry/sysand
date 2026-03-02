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
  sysand.do_new_js_local_storage("basic_new", "1.2.3", "sysand_storage", "/");
  expect(window.localStorage.getItem("sysand_storage/.project.json")).toBe(
    '{"name":"basic_new","version":"1.2.3","usage":[]}',
  );
  expect(window.localStorage.getItem("sysand_storage/.meta.json")).toMatch(
    /\{"index":\{\},"created":"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}.(\d{3}|\d{6}|\d{9})Z"}/,
  );
});

it("can initialise an empty environment in browser local storage", async function () {
  sysand.do_env_js_local_storage("sysand_storage", "/");
  expect(window.localStorage.key(0)).toBe(
    "sysand_storage/sysand_env/entries.txt",
  );
  expect(window.localStorage.key(1)).toBe(null);
  expect(
    window.localStorage.getItem("sysand_storage/sysand_env/entries.txt"),
  ).toBe("");
});
