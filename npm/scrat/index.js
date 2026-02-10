const path = require("path");
const fs = require("fs");

const PLATFORMS = {
  "darwin-arm64": "@claylo/scrat-darwin-arm64",
  "darwin-x64": "@claylo/scrat-darwin-x64",
  "linux-arm64": "@claylo/scrat-linux-arm64",
  "linux-x64": "@claylo/scrat-linux-x64",
  "win32-arm64": "@claylo/scrat-win32-arm64",
  "win32-x64": "@claylo/scrat-win32-x64",
};

function getBinaryPath() {
  const platformKey = `${process.platform}-${process.arch}`;
  const packageName = PLATFORMS[platformKey];

  if (!packageName) {
    throw new Error(`Unsupported platform: ${platformKey}`);
  }

  const binaryName =
    process.platform === "win32" ? "scrat.exe" : "scrat";

  // Try optionalDependency first
  try {
    const packagePath = require.resolve(`${packageName}/package.json`);
    const binaryPath = path.join(path.dirname(packagePath), "bin", binaryName);
    if (fs.existsSync(binaryPath)) {
      return binaryPath;
    }
  } catch {
    // optionalDependency not installed, fall through to fallback
  }

  // Fall back to postinstall location
  const fallbackPath = path.join(__dirname, "bin", binaryName);
  if (fs.existsSync(fallbackPath)) {
    return fallbackPath;
  }

  throw new Error(
    `Could not find scrat binary. ` +
      `Try reinstalling @claylo/scrat`
  );
}

module.exports = { getBinaryPath };
