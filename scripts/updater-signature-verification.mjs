import { createHash, createPublicKey, verify } from "node:crypto";
import { existsSync, lstatSync, readFileSync, realpathSync } from "node:fs";
import { join, resolve } from "node:path";

export const DEVELOPMENT_UPDATER_PUBLIC_KEY =
  "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEVDNTdGQzNFMUNGRkUxQzQKUldURTRmOGNQdnhYN055YjlpWEVscnNSdWhKN3B5cS9WdVRsN1RLOVNSNm16QUhGQzRmN0RsVXIK";

const MINISIGN_PUBLIC_KEY_PATTERN =
  /^untrusted comment: minisign public key: ([0-9A-Fa-f]{1,16})\n(RW[A-Za-z0-9+/]{54})\n?$/u;
const ED25519_SPKI_PREFIX = Buffer.from("302a300506032b6570032100", "hex");
const DEVELOPMENT_UPDATER_RAW_KEY = Buffer.from(
  Buffer.from(DEVELOPMENT_UPDATER_PUBLIC_KEY, "base64")
    .toString("utf8")
    .match(MINISIGN_PUBLIC_KEY_PATTERN)[2],
  "base64",
);

function canonicalBase64(value, label, expectedBytes) {
  if (typeof value !== "string" || !/^[A-Za-z0-9+/]+={0,2}$/u.test(value)) {
    throw new Error(`${label} is not canonical base64.`);
  }
  const decoded = Buffer.from(value, "base64");
  if (decoded.length !== expectedBytes || decoded.toString("base64") !== value) {
    throw new Error(`${label} is not canonical base64.`);
  }
  return decoded;
}

export function normalizeUpdaterPublicKey(value) {
  const publicKey = String(value ?? "").trim().replaceAll(/\r?\n/gu, "");
  const decoded = /^[A-Za-z0-9+/=]+$/u.test(publicKey)
    ? Buffer.from(publicKey, "base64")
    : Buffer.alloc(0);
  const match = decoded.toString("utf8").match(MINISIGN_PUBLIC_KEY_PATTERN);
  if (
    !publicKey
    || publicKey.length > 4096
    || decoded.toString("base64") !== publicKey
    || !match
  ) {
    throw new Error(
      "OOMU_UPDATER_PUBLIC_KEY must be the bounded base64 public key for the dedicated production updater key.",
    );
  }
  const raw = canonicalBase64(match[2], "Updater public key", 42);
  const declaredKeyId = match[1].toLowerCase().padStart(16, "0");
  const encodedKeyId = Buffer.from(raw.subarray(2, 10)).reverse().toString("hex");
  if (raw[0] !== 0x45 || raw[1] !== 0x64) {
    throw new Error("The updater public key uses an unsupported signature algorithm.");
  }
  if (declaredKeyId !== encodedKeyId) {
    throw new Error("The updater public key comment does not match its cryptographic key ID.");
  }
  if (raw.equals(DEVELOPMENT_UPDATER_RAW_KEY)) {
    throw new Error(
      "OOMU_UPDATER_PUBLIC_KEY must be the dedicated production updater key, not the development key.",
    );
  }
  return publicKey;
}

export function updaterPublicKeySha256(value) {
  return createHash("sha256").update(normalizeUpdaterPublicKey(value)).digest("hex");
}

function parsedPublicKey(value) {
  const normalized = normalizeUpdaterPublicKey(value);
  const decodedDocument = Buffer.from(normalized, "base64").toString("utf8");
  const encodedKey = decodedDocument.match(MINISIGN_PUBLIC_KEY_PATTERN)?.[2];
  const raw = canonicalBase64(encodedKey, "Updater public key", 42);
  const keyId = raw.subarray(2, 10);
  const publicKey = createPublicKey({
    key: Buffer.concat([ED25519_SPKI_PREFIX, raw.subarray(10)]),
    format: "der",
    type: "spki",
  });
  return { keyId, publicKey };
}

function parsedSignature(signaturePath) {
  const encoded = readFileSync(signaturePath, "utf8").trim();
  if (Buffer.byteLength(encoded, "utf8") > 16 * 1024) {
    throw new Error("Updater signature exceeds the verified size limit.");
  }
  if (!/^[A-Za-z0-9+/]+={0,2}$/u.test(encoded)) {
    throw new Error("Updater signature is not canonical base64.");
  }
  const decoded = Buffer.from(encoded, "base64");
  if (decoded.toString("base64") !== encoded) {
    throw new Error("Updater signature is not canonical base64.");
  }
  const text = decoded.toString("utf8");
  const lines = text.replace(/\r\n/gu, "\n").replace(/\n$/u, "").split("\n");
  if (
    lines.length !== 4
    || !lines[0].startsWith("untrusted comment: ")
    || !lines[2].startsWith("trusted comment: ")
  ) {
    throw new Error("Updater signature has an unsupported Minisign envelope.");
  }
  const primary = canonicalBase64(lines[1], "Updater signature", 74);
  const global = canonicalBase64(lines[3], "Updater global signature", 64);
  if (primary[0] !== 0x45 || primary[1] !== 0x44) {
    throw new Error("Updater signature must use prehashed Minisign Ed25519.");
  }
  return {
    keyId: primary.subarray(2, 10),
    signature: primary.subarray(10),
    trustedComment: lines[2].slice("trusted comment: ".length),
    global,
  };
}

export function verifyUpdaterArchiveSignature(archivePath, signaturePath, publicKeyValue) {
  const archive = resolve(archivePath);
  const signature = resolve(signaturePath);
  if (
    !existsSync(archive)
    || !lstatSync(archive).isFile()
    || realpathSync(archive) !== archive
    || !existsSync(signature)
    || !lstatSync(signature).isFile()
    || realpathSync(signature) !== signature
  ) {
    throw new Error("Updater signature verification requires exact real files.");
  }
  const key = parsedPublicKey(publicKeyValue);
  const envelope = parsedSignature(signature);
  if (!key.keyId.equals(envelope.keyId)) {
    throw new Error("Updater signature was created by a different key.");
  }
  const payloadHash = createHash("blake2b512").update(readFileSync(archive)).digest();
  if (!verify(null, payloadHash, key.publicKey, envelope.signature)) {
    throw new Error("Updater archive signature does not match OOMU_UPDATER_PUBLIC_KEY.");
  }
  const globalPayload = Buffer.concat([
    envelope.signature,
    Buffer.from(envelope.trustedComment, "utf8"),
  ]);
  if (!verify(null, globalPayload, key.publicKey, envelope.global)) {
    throw new Error("Updater signature trusted comment failed verification.");
  }
  return true;
}

export function assertUpdaterPublicKeyEmbeddedInApp(appPath, publicKeyValue) {
  const app = resolve(appPath);
  const executable = join(app, "Contents", "MacOS", "oomu");
  if (
    !existsSync(app)
    || !lstatSync(app).isDirectory()
    || realpathSync(app) !== app
    || !existsSync(executable)
    || !lstatSync(executable).isFile()
    || realpathSync(executable) !== executable
  ) {
    throw new Error("The qualified OOMU application executable is missing or indirect.");
  }
  const publicKey = normalizeUpdaterPublicKey(publicKeyValue);
  if (!readFileSync(executable).includes(Buffer.from(publicKey, "utf8"))) {
    throw new Error("The qualified OOMU application does not embed OOMU_UPDATER_PUBLIC_KEY.");
  }
  return true;
}
