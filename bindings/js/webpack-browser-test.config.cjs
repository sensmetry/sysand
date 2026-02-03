const path = require("path");
//const HtmlWebpackPlugin = require('html-webpack-plugin');
//const webpack = require('webpack');
const glob = require("glob");

module.exports = {
  entry: glob.sync("spec/browser/*.spec.js").map((x) => "./" + x), //.concat(["./shiv.js"]),
  output: {
    path: path.resolve(__dirname, "browser_test_dist"),
    filename: "bundle.js",
  },
  // plugins: [
  //     //new HtmlWebpackPlugin(),
  //     new WasmPackPlugin({
  //         crateDirectory: path.resolve(__dirname),
  //         outDir: path.resolve(__dirname, "browser_pkg")
  //     }),
  // ],
  resolve: {
    modules: ["node_modules"],
    extensions: ["*", ".js", ".jsx", ".tsx", ".ts"],
    alias: {
      sysand$: path.resolve(__dirname, "browser_pkg/index.js"),
    },
  },
  mode: "development", // mode: 'production'
  experiments: {
    asyncWebAssembly: true,
  },
};
