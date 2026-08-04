import { afterEach, describe, expect, it } from "vitest";
import { spawnSync } from "node:child_process";
import {
  mkdirSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import {
  DEVELOPMENT_UPDATER_PUBLIC_KEY,
  assertUpdaterPublicKeyEmbeddedInApp,
  normalizeUpdaterPublicKey,
  verifyUpdaterArchiveSignature,
} from "../updater-signature-verification.mjs";

const root = resolve(import.meta.dirname, "..", "..");
const tauri = join(root, "node_modules", ".bin", "tauri");
const temporaryDirectories = [];

afterEach(() => {
  while (temporaryDirectories.length) {
    rmSync(temporaryDirectories.pop(), { recursive: true, force: true });
  }
});

function temporaryDirectory() {
  const directory = realpathSync(mkdtempSync(join(tmpdir(), "oomu-updater-binding-")));
  temporaryDirectories.push(directory);
  return directory;
}

function runSigner(args) {
  const result = spawnSync(tauri, ["signer", ...args], {
    cwd: root,
    encoding: "utf8",
    env: process.env,
  });
  if (result.status !== 0) {
    throw new Error(`Tauri signer failed during the cryptographic binding test: ${result.stderr}`);
  }
}

function generateKey(directory, name) {
  const privateKey = join(directory, `${name}.key`);
  runSigner([
    "generate", "--ci", "--password", "test-only", "--write-keys", privateKey,
  ]);
  return {
    privateKey,
    publicKey: readFileSync(`${privateKey}.pub`, "utf8"),
  };
}

describe("updater cryptographic release binding", () => {
  it("rejects the development key by raw key material, not editable comments", () => {
    expect(() => normalizeUpdaterPublicKey(DEVELOPMENT_UPDATER_PUBLIC_KEY))
      .toThrow(/development key|dedicated production/u);
    const developmentDocument = Buffer.from(
      DEVELOPMENT_UPDATER_PUBLIC_KEY,
      "base64",
    ).toString("utf8");
    const relabeledDevelopmentKey = Buffer.from(
      developmentDocument.replace(
        /minisign public key: [0-9A-F]{16}/u,
        "minisign public key: 0000000000000000",
      ),
      "utf8",
    ).toString("base64");
    expect(() => normalizeUpdaterPublicKey(relabeledDevelopmentKey))
      .toThrow(/development key|key ID/u);
  });

  it("accepts Minisign key comments that canonically omit a leading zero", () => {
    const keyId = Buffer.from("0807060504030201", "hex");
    const raw = Buffer.concat([
      Buffer.from([0x45, 0x64]),
      keyId,
      Buffer.alloc(32, 0x5a),
    ]);
    const displayedId = Buffer.from(keyId)
      .reverse()
      .toString("hex")
      .toUpperCase()
      .replace(/^0+/u, "");
    const document = [
      `untrusted comment: minisign public key: ${displayedId}`,
      raw.toString("base64"),
      "",
    ].join("\n");
    const encoded = Buffer.from(document, "utf8").toString("base64");
    expect(normalizeUpdaterPublicKey(encoded)).toBe(encoded);
  });

  it("accepts only an archive signature made by the matching updater key", () => {
    const directory = temporaryDirectory();
    const first = generateKey(directory, "first");
    const second = generateKey(directory, "second");
    const archive = join(directory, "OOMU_0.1.3_darwin-aarch64.app.tar.gz");
    writeFileSync(archive, "exact updater archive bytes\n");
    runSigner([
      "sign", "--private-key-path", first.privateKey,
      "--password", "test-only", archive,
    ]);

    expect(verifyUpdaterArchiveSignature(archive, `${archive}.sig`, first.publicKey)).toBe(true);
    expect(() => verifyUpdaterArchiveSignature(archive, `${archive}.sig`, second.publicKey))
      .toThrow(/different key/u);
    writeFileSync(archive, "changed updater archive bytes\n");
    expect(() => verifyUpdaterArchiveSignature(archive, `${archive}.sig`, first.publicKey))
      .toThrow(/does not match/u);
  });

  it("requires the same production updater public key inside the exact app executable", () => {
    const directory = temporaryDirectory();
    const first = generateKey(directory, "embedded");
    const second = generateKey(directory, "other");
    const app = join(directory, "OOMU.app");
    const executableDirectory = join(app, "Contents", "MacOS");
    mkdirSync(executableDirectory, { recursive: true });
    writeFileSync(
      join(executableDirectory, "oomu"),
      Buffer.concat([
        Buffer.from("binary-prefix\0"),
        Buffer.from(normalizeUpdaterPublicKey(first.publicKey)),
        Buffer.from("\0binary-suffix"),
      ]),
    );

    expect(assertUpdaterPublicKeyEmbeddedInApp(app, first.publicKey)).toBe(true);
    expect(() => assertUpdaterPublicKeyEmbeddedInApp(app, second.publicKey))
      .toThrow(/does not embed/u);
  });
});
