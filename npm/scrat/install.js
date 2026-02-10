const https = require("https");
const fs = require("fs");
const path = require("path");
const zlib = require("zlib");

const PLATFORMS = {
  "darwin-arm64": "@claylo/scrat-darwin-arm64",
  "darwin-x64": "@claylo/scrat-darwin-x64",
  "linux-arm64": "@claylo/scrat-linux-arm64",
  "linux-x64": "@claylo/scrat-linux-x64",
  "win32-arm64": "@claylo/scrat-win32-arm64",
  "win32-x64": "@claylo/scrat-win32-x64",
};

async function install() {
  const platformKey = `${process.platform}-${process.arch}`;
  const packageName = PLATFORMS[platformKey];

  if (!packageName) {
    console.warn(`Unsupported platform: ${platformKey}`);
    return;
  }

  // Check if optionalDependency already installed
  try {
    require.resolve(`${packageName}/package.json`);
    return; // Already installed
  } catch {
    // Not installed, proceed with download
  }

  console.log(`Downloading ${packageName}...`);

  const version = require("./package.json").version;
  const tarballUrl = `https://registry.npmjs.org/${packageName}/-/${packageName.split("/")[1]}-${version}.tgz`;

  const tarball = await download(tarballUrl);
  const files = extractTar(zlib.gunzipSync(tarball));

  const binaryName =
    process.platform === "win32" ? "scrat.exe" : "scrat";
  const binaryEntry = files.find((f) => f.name.endsWith(`/bin/${binaryName}`));

  if (!binaryEntry) {
    throw new Error("Binary not found in package");
  }

  const binDir = path.join(__dirname, "bin");
  fs.mkdirSync(binDir, { recursive: true });
  fs.writeFileSync(path.join(binDir, binaryName), binaryEntry.data, {
    mode: 0o755,
  });
}

function download(url) {
  return new Promise((resolve, reject) => {
    https.get(url, (res) => {
      if (res.statusCode === 302 || res.statusCode === 301) {
        return download(res.headers.location).then(resolve, reject);
      }
      const chunks = [];
      res.on("data", (chunk) => chunks.push(chunk));
      res.on("end", () => resolve(Buffer.concat(chunks)));
      res.on("error", reject);
    });
  });
}

function extractTar(buffer) {
  const files = [];
  let offset = 0;

  while (offset < buffer.length - 512) {
    const header = buffer.slice(offset, offset + 512);
    if (header[0] === 0) break;

    const name = header.slice(0, 100).toString().replace(/\0/g, "");
    const size = parseInt(header.slice(124, 136).toString(), 8);

    offset += 512;
    if (size > 0) {
      files.push({ name, data: buffer.slice(offset, offset + size) });
      offset += Math.ceil(size / 512) * 512;
    }
  }

  return files;
}

install().catch((err) => {
  console.error("Failed to install binary:", err.message);
  process.exit(1);
});
