// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

import {
  copyFileSync,
  cpSync,
  existsSync,
  readdirSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
import { join } from "node:path";

const outputDir = "_build/html";
const sourceDir = "assets/favicons";
const targetDir = join(outputDir, "favicons");
const staticFiles = ["install.sh", "install.ps1"];

if (!existsSync(outputDir)) {
  throw new Error(`Expected ${outputDir} to exist. Run myst build first.`);
}

cpSync(sourceDir, targetDir, { recursive: true });

for (const file of staticFiles) {
  copyFileSync(file, join(outputDir, file));
}

const faviconHead = [
  '<link rel="icon" href="/favicons/favicon-96.png" type="image/png" sizes="96x96"/>',
  '<link rel="icon" href="/favicons/favicon.ico" sizes="any"/>',
  '<link rel="icon" href="/favicons/favicon.svg" type="image/svg+xml"/>',
  '<link rel="apple-touch-icon" sizes="180x180" href="/favicons/apple-touch-icon.png"/>',
  '<link rel="manifest" href="/favicons/site.webmanifest"/>',
  '<link rel="mask-icon" href="/favicons/safari-pinned-tab.svg" color="#ed8936"/>',
  '<meta name="theme-color" content="#0d1117"/>',
].join("");

// MyST treats absolute URLs as external links and opens them in a new tab.
// These Sysand-owned top-nav links are part of the same product surface, so
// they should behave like normal navigation between Sysand web properties.
const sameWindowNavLinks = [
  { title: "Index docs", url: "https://docs.sysand.com" },
  { title: "Client docs", url: "https://client.sysand.com" },
  { title: "Index", url: "https://sysand.com" },
];

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function normalizeNavLinks(html) {
  let updatedHtml = html;
  // Limit the rewrite to exact title/url pairs so arbitrary external links in
  // page content keep their default new-tab behavior.
  for (const { title, url } of sameWindowNavLinks) {
    const pattern = new RegExp(
      `(<a href="${escapeRegExp(url)}") target="_blank" rel="noopener noreferrer"([^>]*>${escapeRegExp(title)}</a>)`,
      "g",
    );
    updatedHtml = updatedHtml.replace(pattern, "$1$2");
  }
  return updatedHtml;
}

function htmlFiles(dir) {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return htmlFiles(path);
    return entry.isFile() && entry.name.endsWith(".html") ? [path] : [];
  });
}

for (const file of htmlFiles(outputDir)) {
  let html = readFileSync(file, "utf8");
  if (!html.includes('rel="apple-touch-icon"')) {
    html = html.replace("</head>", `${faviconHead}</head>`);
  }
  writeFileSync(file, normalizeNavLinks(html));
}
