// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

const path = require("path");
//const HtmlWebpackPlugin = require('html-webpack-plugin');
//const webpack = require('webpack');
const WasmPackPlugin = require("@wasm-tool/wasm-pack-plugin");
// const glob = require("glob");

module.exports = {
  //entry: glob.sync("tests/*.spec.js").map((x) => './' + x), //.concat(["./shiv.js"]),
  entry: ["./browser_pkg/index.js"],
  output: {
    path: path.resolve(__dirname, "browser_dist"),
    filename: "bundle.js",
  },
  plugins: [
    //new HtmlWebpackPlugin(),
    new WasmPackPlugin({
      crateDirectory: path.resolve(__dirname),
      outDir: path.resolve(__dirname, "browser_pkg"),
    }),
  ],
  resolve: {
    modules: ["node_modules"],
    extensions: ["*", ".js", ".jsx", ".tsx", ".ts"],
  },
  mode: "development", // mode: 'production'
  experiments: {
    asyncWebAssembly: true,
  },
};
