export default {
  srcDir: "browser_test_dist",
  srcFiles: [
    //"**/*.js"
  ],
  specDir: "browser_test_dist",
  specFiles: [
    "bundle.js"
  ],
  helpers: [
    //"helpers/**/*.js"
  ],
  env: {
    stopSpecOnExpectationFailure: false,
    stopOnSpecFailure: false,
    random: true,
    // Fail if a suite contains multiple suites or specs with the same name.
    forbidDuplicateNames: true
  },

  enableTopLevelAwait: true,

  // For security, listen only to localhost. You can also specify a different
  // hostname or IP address, or remove the property or set it to "*" to listen
  // to all network interfaces.
  listenAddress: "localhost",

  // The hostname that the browser will use to connect to the server.
  hostname: "localhost",

  browser: {
    name: "headlessFirefox"
  }
};
