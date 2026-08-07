import { createHash } from "node:crypto";
import {
  mkdtempSync,
  readFileSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { detachDmgFromCreationHelper } from "../release.mjs";

const temporaryDirectories = [];

afterEach(() => {
  for (const directory of temporaryDirectories.splice(0)) {
    rmSync(directory, { recursive: true, force: true });
  }
});

describe("release DMG detachment", () => {
  it("preserves signed bytes while replacing the creation inode", () => {
    const directory = mkdtempSync(join(tmpdir(), "oomu-dmg-detach-"));
    temporaryDirectories.push(directory);
    const dmgPath = join(directory, "OOMU.dmg");
    const contents = Buffer.from("signed-dmg-test-bytes");
    writeFileSync(dmgPath, contents);
    const originalInode = statSync(dmgPath).ino;
    const expectedDigest = createHash("sha256").update(contents).digest("hex");

    expect(detachDmgFromCreationHelper(dmgPath)).toBe(expectedDigest);
    expect(readFileSync(dmgPath)).toEqual(contents);
    expect(statSync(dmgPath).ino).not.toBe(originalInode);
  });
});
