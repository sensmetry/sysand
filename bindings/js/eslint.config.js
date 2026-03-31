import js from "@eslint/js";
import globals from "globals";
import { defineConfig } from "eslint/config";

export default defineConfig([
  {
    files: ["**/*.{js,mjs,cjs}"],
    ignores: [
      "browser_dist/**/*.js",
      "browser_pkg/**/*.js",
      "browser_test_dist/**/*.js",
    ],
    plugins: { js },
    extends: ["js/recommended"],
    languageOptions: {
      globals: { ...globals.browser, ...globals.jasmine, ...globals.node },
    },
  },
]);
