import { createRequire } from "module";

const require = createRequire(import.meta.url);

const xmlPlugin = await import(require.resolve("@prettier/plugin-xml"));
const javaPlugin = await import(require.resolve("prettier-plugin-java"));

export default {
  plugins: [xmlPlugin.default, javaPlugin.default],
  overrides: [
    {
      files: "*.java",
      options: {
        tabWidth: 4,
      },
    },
  ],
};
