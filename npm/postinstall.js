#!/usr/bin/env node
"use strict";

const { execSync } = require("child_process");
const fs = require("fs");
const https = require("https");
const path = require("path");
const { pipeline } = require("stream/promises");
const { createGunzip } = require("zlib");

const VERSION = require("./package.json").version;

const TARGETS = {
  "darwin-x64": "x86_64-apple-darwin",
  "darwin-arm64": "aarch64-apple-darwin",
  "linux-x64": "x86_64-unknown-linux-musl",
  "linux-arm64": "aarch64-unknown-linux-musl",
  "win32-x64": "x86_64-pc-windows-msvc",
};

function getTarget() {
  const key = `${process.platform}-${process.arch}`;
  const target = TARGETS[key];
  if (!target) {
    console.error(
      `Unsupported platform: ${process.platform}-${process.arch}`
    );
    console.error(
      "Download manually from https://github.com/remit-md/remit-cli/releases"
    );
    process.exit(1);
  }
  return target;
}

function getAssetUrl(target) {
  const ext = process.platform === "win32" ? "zip" : "tar.gz";
  return `https://github.com/remit-md/remit-cli/releases/download/v${VERSION}/remit-${target}.${ext}`;
}

function downloadFile(url) {
  return new Promise((resolve, reject) => {
    const follow = (url, redirects) => {
      if (redirects > 5) return reject(new Error("Too many redirects"));
      https
        .get(url, (res) => {
          if (res.statusCode >= 300 && res.statusCode < 400 && res.headers.location) {
            return follow(res.headers.location, redirects + 1);
          }
          if (res.statusCode !== 200) {
            return reject(
              new Error(`Download failed: HTTP ${res.statusCode} from ${url}`)
            );
          }
          const chunks = [];
          res.on("data", (chunk) => chunks.push(chunk));
          res.on("end", () => resolve(Buffer.concat(chunks)));
          res.on("error", reject);
        })
        .on("error", reject);
    };
    follow(url, 0);
  });
}

async function extractTarGz(buffer, destDir) {
  // Write to temp file, extract with tar
  const tmpFile = path.join(destDir, ".remit-download.tar.gz");
  fs.writeFileSync(tmpFile, buffer);
  try {
    execSync(`tar xzf "${tmpFile}" -C "${destDir}"`, { stdio: "pipe" });
  } finally {
    try {
      fs.unlinkSync(tmpFile);
    } catch (_) {}
  }
}

async function extractZip(buffer, destDir) {
  // Write to temp file, extract with PowerShell
  const tmpFile = path.join(destDir, ".remit-download.zip");
  fs.writeFileSync(tmpFile, buffer);
  try {
    execSync(
      `powershell -Command "Expand-Archive -Force '${tmpFile}' '${destDir}'"`,
      { stdio: "pipe" }
    );
  } finally {
    try {
      fs.unlinkSync(tmpFile);
    } catch (_) {}
  }
}

async function main() {
  const target = getTarget();
  const url = getAssetUrl(target);
  const binDir = path.join(__dirname, "bin");
  const binName = process.platform === "win32" ? "remit.exe" : "remit";
  const binPath = path.join(binDir, binName);

  // Skip if already installed at correct version
  if (fs.existsSync(binPath)) {
    try {
      const output = execSync(`"${binPath}" --version`, {
        encoding: "utf8",
        timeout: 5000,
      }).trim();
      if (output.includes(VERSION)) {
        return;
      }
    } catch (_) {
      // Binary exists but broken — re-download
    }
  }

  console.log(`Downloading remit v${VERSION} for ${process.platform}-${process.arch}...`);
  const buffer = await downloadFile(url);

  fs.mkdirSync(binDir, { recursive: true });

  if (process.platform === "win32") {
    await extractZip(buffer, binDir);
  } else {
    await extractTarGz(buffer, binDir);
  }

  // Set executable permission on Unix
  if (process.platform !== "win32") {
    fs.chmodSync(binPath, 0o755);
  }

  // Verify
  try {
    const output = execSync(`"${binPath}" --version`, {
      encoding: "utf8",
      timeout: 5000,
    }).trim();
    console.log(`Installed: ${output}`);
  } catch (err) {
    console.error("Warning: installed binary failed version check:", err.message);
  }
}

main().catch((err) => {
  console.error("postinstall failed:", err.message);
  console.error(
    "Download manually from https://github.com/remit-md/remit-cli/releases"
  );
  // Don't fail npm install — the bin/remit stub will print instructions
  process.exit(0);
});
