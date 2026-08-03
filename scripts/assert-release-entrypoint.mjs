#!/usr/bin/env node

import { createHash, createPublicKey, verify as verifySignature } from "node:crypto";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import process from "node:process";
import { TRUSTED_RELEASE_PUBLIC_KEY_HEX } from "./release-manifest.mjs";

export function verifyReleaseAuthorization({
  buildId,
  sourceRevision,
  signatureBase64,
  publicKeyPath,
  trustedPublicKeyHex = TRUSTED_RELEASE_PUBLIC_KEY_HEX,
}) {
  if (
    !buildId?.match(/^[A-Za-z0-9._-]{8,128}$/) ||
    !sourceRevision?.match(/^[0-9a-f]{40}$/i) ||
    !signatureBase64 ||
    !publicKeyPath
  ) {
    return false;
  }
  const publicKey = createPublicKey(readFileSync(resolve(publicKeyPath)));
  if (publicKey.asymmetricKeyType !== "ed25519") return false;
  const rawPublicKey = Buffer.from(publicKey.export({ format: "jwk" }).x, "base64url");
  if (rawPublicKey.toString("hex") !== trustedPublicKeyHex) return false;
  const payload = Buffer.from(`oomu-release-v1\n${buildId}\n${sourceRevision}`, "utf8");
  return verifySignature(
    null,
    payload,
    publicKey,
    Buffer.from(signatureBase64, "base64"),
  );
}

function authorizedEnvironment() {
  if (process.env.OOMU_LOCAL_UNSIGNED_BUILD === "1") return true;
  if (process.env.OOMU_RELEASE_PIPELINE === "unsigned-v2") {
    const policy = readFileSync(resolve("release/release-policy.json"));
    const policyDigest = createHash("sha256").update(policy).digest("hex");
    return (
      process.env.GITHUB_ACTIONS === "true" &&
      process.env.OOMU_RELEASE_POLICY_SHA256 === policyDigest &&
      /^[A-Za-z0-9._-]{8,128}$/u.test(process.env.OOMU_BUILD_ID ?? "") &&
      /^[0-9a-f]{40}$/iu.test(process.env.OOMU_SOURCE_REVISION ?? "")
    );
  }
  return (
    process.env.OOMU_RELEASE_PIPELINE === "canonical-v1" &&
    verifyReleaseAuthorization({
      buildId: process.env.OOMU_BUILD_ID?.trim(),
      sourceRevision: process.env.OOMU_SOURCE_REVISION?.trim(),
      signatureBase64: process.env.OOMU_RELEASE_AUTHORIZATION_BASE64?.trim(),
      publicKeyPath: process.env.OOMU_RELEASE_MANIFEST_PUBLIC_KEY_PATH?.trim(),
    })
  );
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  try {
    if (!authorizedEnvironment()) throw new Error("release authorization is absent or invalid");
  } catch {
    console.error(
      "OOMU RELEASE ERROR: distributable Tauri steps are internal. Run `npm run build:prod`.",
    );
    process.exit(1);
  }
}
