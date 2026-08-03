#!/usr/bin/env node

import { createHash } from "node:crypto";
import { createReadStream, readFileSync, statSync, writeFileSync, mkdirSync } from "node:fs";
import https from "node:https";
import { basename, join, resolve } from "node:path";
import { pipeline } from "node:stream/promises";
import process from "node:process";

const MAX_RESPONSE_BYTES = 8 * 1024 * 1024;
const MAX_ARTIFACT_BYTES = 2 * 1024 * 1024 * 1024;
const ALLOWED_JOBS = new Set(["clean-machine-launch", "p0-acceptance"]);

function requiredEnvironment(name) {
  const value = process.env[name]?.trim();
  if (!value) {
    throw new Error(
      `not executed - environment not ready: ${name} is required for the real release lab`,
    );
  }
  return value;
}

function releaseLabUrl(job) {
  const base = new URL(requiredEnvironment("OOMU_RELEASE_LAB_URL"));
  if (
    base.protocol !== "https:" ||
    base.port !== "" ||
    base.username !== "" ||
    base.password !== "" ||
    base.search !== "" ||
    base.hash !== ""
  ) {
    throw new Error("The release lab URL must be a reviewed HTTPS origin.");
  }
  return new URL(`/v1/release-jobs/${job}`, base);
}

export function parseRunnerArguments(argv) {
  const values = {};
  for (let index = 0; index < argv.length; index += 2) {
    const key = argv[index];
    const value = argv[index + 1];
    if (!key?.startsWith("--") || !value) throw new Error("Release runner arguments are malformed.");
    values[key.slice(2)] = value;
  }
  return values;
}

async function sha256File(path) {
  const digest = createHash("sha256");
  for await (const chunk of createReadStream(path)) digest.update(chunk);
  return digest.digest("hex");
}

function boundedMetadata(job, args, artifactPath, artifactSha256) {
  const metadata = {
    schemaVersion: 1,
    job,
    artifactName: basename(artifactPath),
    artifactSha256,
    request: Object.fromEntries(
      Object.entries(args).filter(([key]) => !["artifact", "output", "output-dir"].includes(key)),
    ),
  };
  const encoded = Buffer.from(JSON.stringify(metadata), "utf8").toString("base64");
  if (encoded.length > 16 * 1024) throw new Error("Release runner metadata is too large.");
  return encoded;
}

async function postArtifact(job, args) {
  if (!ALLOWED_JOBS.has(job)) throw new Error("Release runner job is not allowed.");
  const artifactPath = resolve(args.artifact ?? "");
  const size = statSync(artifactPath).size;
  if (size <= 0 || size > MAX_ARTIFACT_BYTES) throw new Error("Release artifact size is invalid.");
  const artifactSha256 = await sha256File(artifactPath);
  return new Promise((resolvePromise, reject) => {
    const request = https.request(
      releaseLabUrl(job),
      {
        method: "POST",
        minVersion: "TLSv1.3",
        cert: readFileSync(resolve(requiredEnvironment("OOMU_RELEASE_LAB_CLIENT_CERT_PATH"))),
        key: readFileSync(resolve(requiredEnvironment("OOMU_RELEASE_LAB_CLIENT_KEY_PATH"))),
        ca: readFileSync(resolve(requiredEnvironment("OOMU_RELEASE_LAB_CA_PATH"))),
        rejectUnauthorized: true,
        timeout: 30 * 60 * 1000,
        headers: {
          "Content-Type": "application/octet-stream",
          "Content-Length": String(size),
          "X-OOMU-Release-Request": boundedMetadata(job, args, artifactPath, artifactSha256),
        },
      },
      (response) => {
        const chunks = [];
        let received = 0;
        response.on("data", (chunk) => {
          received += chunk.length;
          if (received > MAX_RESPONSE_BYTES) request.destroy(new Error("Release lab response is too large."));
          else chunks.push(chunk);
        });
        response.on("end", () => {
          if (response.statusCode !== 200) return reject(new Error("The release lab rejected the job."));
          try {
            const decoded = JSON.parse(Buffer.concat(chunks).toString("utf8"));
            if (
              decoded.schemaVersion !== 1 ||
              decoded.job !== job ||
              decoded.status !== "passed" ||
              decoded.synthetic !== false ||
              decoded.artifactSha256 !== artifactSha256
            ) {
              throw new Error("Release lab evidence does not match the submitted artifact.");
            }
            resolvePromise(decoded);
          } catch (error) {
            reject(error);
          }
        });
      },
    );
    request.on("timeout", () => request.destroy(new Error("The release lab timed out.")));
    request.on("error", reject);
    pipeline(createReadStream(artifactPath), request).catch(reject);
  });
}

function writeOutput(job, args, response) {
  if (job === "p0-acceptance") {
    const outputDir = resolve(args["output-dir"] ?? "");
    mkdirSync(outputDir, { recursive: true, mode: 0o700 });
    if (!Array.isArray(response.files) || response.files.length === 0 || response.files.length > 32) {
      throw new Error("P0 release lab evidence files are missing.");
    }
    for (const file of response.files) {
      if (!/^[a-z0-9][a-z0-9._-]{1,127}\.json$/.test(file.name ?? "")) {
        throw new Error("P0 release lab returned an unsafe evidence filename.");
      }
      const bytes = Buffer.from(file.contentBase64 ?? "", "base64");
      if (bytes.length > 1024 * 1024 || createHash("sha256").update(bytes).digest("hex") !== file.sha256) {
        throw new Error("P0 release lab evidence integrity failed.");
      }
      writeFileSync(join(outputDir, file.name), bytes, { mode: 0o600 });
    }
    return;
  }
  if (!args.output || typeof response.evidence !== "object" || response.evidence === null) {
    throw new Error("Release lab evidence output is missing.");
  }
  writeFileSync(resolve(args.output), `${JSON.stringify(response.evidence, null, 2)}\n`, { mode: 0o600 });
}

export async function runRemoteReleaseJob(job, argv) {
  const args = parseRunnerArguments(argv);
  if (!args.artifact) throw new Error("--artifact is required.");
  const response = await postArtifact(job, args);
  writeOutput(job, args, response);
}
