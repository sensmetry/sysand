const js = require("@eslint/js");
const globals = require("globals");

module.exports = [
  {
    files: ["**/*.{js,mjs,cjs}"],
    ignores: [
      "**/browser_dist/**/*.js",
      "**/browser_pkg/**/*.js",
      "**/browser_test_dist/**/*.js",
    ],
    plugins: { js },
    ...js.configs.recommended,
    languageOptions: {
      globals: { ...globals.browser, ...globals.jasmine, ...globals.node },
    },
  },
];
