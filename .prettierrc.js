module.exports = {
  plugins: [
    require("@prettier/plugin-xml").default,
    require("prettier-plugin-java").default,
  ],
  overrides: [
    {
      files: "*.java",
      options: {
        tabWidth: 4,
      },
    },
  ],
};
