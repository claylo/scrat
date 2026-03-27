const https = require("https");
const fs = require("fs");
const path = require("path");
const zlib = require("zlib");

const MAX_REDIRECTS = 5;
const MAX_DOWNLOAD_BYTES = 50 * 1024 * 1024; // 50 MiB
const REQUEST_TIMEOUT_MS = 30_000; // 30 seconds

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

function download(url, redirectCount = 0) {
  return new Promise((resolve, reject) => {
    if (redirectCount > MAX_REDIRECTS) {
      return reject(new Error(`Too many redirects (>${MAX_REDIRECTS})`));
    }

    const req = https.get(url, (res) => {
      if (res.statusCode === 301 || res.statusCode === 302) {
        const location = res.headers.location;
        if (!location || !location.startsWith("https://")) {
          return reject(
            new Error(`Redirect to non-HTTPS URL: ${location ?? "(empty)"}`),
          );
        }
        res.resume(); // drain the response
        return download(location, redirectCount + 1).then(resolve, reject);
      }

      if (res.statusCode < 200 || res.statusCode >= 300) {
        res.resume();
        return reject(
          new Error(`HTTP ${res.statusCode} from ${url}`),
        );
      }

      const chunks = [];
      let totalBytes = 0;
      res.on("data", (chunk) => {
        totalBytes += chunk.length;
        if (totalBytes > MAX_DOWNLOAD_BYTES) {
          res.destroy();
          return reject(
            new Error(
              `Download exceeds ${MAX_DOWNLOAD_BYTES} bytes limit`,
            ),
          );
        }
        chunks.push(chunk);
      });
      res.on("end", () => resolve(Buffer.concat(chunks)));
      res.on("error", reject);
    });

    req.on("error", reject);
    req.setTimeout(REQUEST_TIMEOUT_MS, () => {
      req.destroy();
      reject(new Error(`Request timed out after ${REQUEST_TIMEOUT_MS}ms`));
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
      // Reject path traversal and absolute paths
      if (name.startsWith("/") || name.split("/").includes("..")) {
        throw new Error(`Unsafe tar entry path: ${name}`);
      }
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
