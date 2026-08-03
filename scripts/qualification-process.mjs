#!/usr/bin/env node

import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { existsSync, readFileSync, realpathSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import process from "node:process";
import { atomicWriteFile } from "./release-manifest.mjs";

export const CLEANUP_RECEIPT_KIND = "exact_process_cleanup";
export const CLEANUP_RECEIPT_SCHEMA_VERSION = 1;

function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}

function sleep(ms) {
  return new Promise((resolveSleep) => setTimeout(resolveSleep, ms));
}

function loadedExecutable(pid, command) {
  const result = spawnSync(
    "/usr/sbin/lsof",
    ["-a", "-p", String(pid), "-d", "txt", "-F", "nDi"],
    { encoding: "utf8", env: { ...process.env, LC_ALL: "C" } },
  );
  const fields = result.status === 0 ? result.stdout.split(/\r?\n/u) : [];
  const nameIndex = fields.findIndex((line) => line.startsWith("n/"));
  const candidate = nameIndex >= 0 ? fields[nameIndex].slice(1) : command;
  const executable = existsSync(candidate)
    ? realpathSync(candidate)
    : (existsSync(command) ? realpathSync(command) : candidate);
  const entryStart = nameIndex < 0
    ? -1
    : fields.slice(0, nameIndex).findLastIndex((line) => line === "ftxt");
  const entry = entryStart < 0 ? [] : fields.slice(entryStart, nameIndex + 1);
  const device = entry.find((line) => line.startsWith("D"))?.slice(1) ?? "";
  const inode = entry.find((line) => line.startsWith("i"))?.slice(1) ?? "";
  return {
    executable,
    loadedImageIdentitySha256: device && inode
      ? sha256(`${device}:${inode}`)
      : sha256(executable),
  };
}

export function parseProcessSnapshot(output) {
  const value = output.trim();
  if (!value) return null;
  const match = value.match(
    /^(\d+)\s+(\d+)\s+(\d+)\s+(\S+)\s+([A-Z][a-z]{2}\s+[A-Z][a-z]{2}\s+\d{1,2}\s+\d{2}:\d{2}:\d{2}\s+\d{4})\s+(.+)$/u,
  );
  if (!match) throw new Error("Unable to parse exact process identity.");
  if (match[4].includes("Z")) return null;
  return {
    pid: Number(match[1]),
    parentPid: Number(match[2]),
    processGroupId: Number(match[3]),
    processState: match[4],
    launchTime: match[5],
    command: match[6],
  };
}

export function inspectProcess(pid) {
  if (!Number.isSafeInteger(pid) || pid <= 1) return null;
  const result = spawnSync(
    "/bin/ps",
    ["-ww", "-p", String(pid), "-o", "pid=", "-o", "ppid=", "-o", "pgid=",
      "-o", "stat=", "-o", "lstart=", "-o", "comm="],
    { encoding: "utf8", env: { ...process.env, LC_ALL: "C" } },
  );
  if (result.status !== 0 || !result.stdout.trim()) return null;
  const snapshot = parseProcessSnapshot(result.stdout);
  if (!snapshot) return null;
  const loaded = loadedExecutable(pid, snapshot.command);
  const { executable } = loaded;
  return {
    pid: snapshot.pid,
    parentPid: snapshot.parentPid,
    processGroupId: snapshot.processGroupId,
    processState: snapshot.processState,
    launchTime: snapshot.launchTime,
    executable,
    executableIdentitySha256: sha256(executable),
    loadedImageIdentitySha256: loaded.loadedImageIdentitySha256,
    executableSha256: existsSync(executable)
      ? sha256(readFileSync(executable))
      : null,
  };
}

export function sameOwnedProcess(binding, current) {
  return Boolean(
    current &&
    binding?.pid === current.pid &&
    binding?.processGroupId === current.processGroupId &&
    binding?.launchTime === current.launchTime &&
    binding?.loadedImageIdentitySha256 === current.loadedImageIdentitySha256,
  );
}

export function sameProcessLifetime(binding, current) {
  return Boolean(
    current
    && binding?.pid === current.pid
    && binding?.processGroupId === current.processGroupId
    && binding?.launchTime === current.launchTime,
  );
}

export function captureOwnedProcess(pid, expectedExecutable) {
  const binding = inspectProcess(pid);
  if (!binding) throw new Error(`Qualification process ${pid} is not running.`);
  if (expectedExecutable) {
    const expected = realpathSync(resolve(expectedExecutable));
    if (binding.executable !== expected) {
      throw new Error(`Qualification process ${pid} is not the expected executable.`);
    }
  }
  return binding;
}

export async function stopOwnedProcess(
  binding,
  {
    inspect = inspectProcess,
    signal = process.kill.bind(process),
    gracefulTimeoutMs = 5_000,
    forcedTimeoutMs = 2_000,
    pollMs = 50,
  } = {},
) {
  const startedAt = new Date().toISOString();
  const initial = inspect(binding.pid);
  if (!initial) return cleanupReceipt(binding, startedAt, "already_stopped", false);
  if (!sameOwnedProcess(binding, initial)) {
    throw new Error("Qualification cleanup refused to signal a reused or mismatched PID.");
  }

  signal(binding.pid, "SIGTERM");
  if (await waitForExit(binding, inspect, gracefulTimeoutMs, pollMs)) {
    return cleanupReceipt(binding, startedAt, "graceful", false);
  }

  const beforeForce = inspect(binding.pid);
  if (!sameProcessLifetime(binding, beforeForce)) {
    throw new Error("Qualification cleanup refused a forced signal after process identity changed.");
  }
  signal(binding.pid, "SIGKILL");
  if (!(await waitForExit(binding, inspect, forcedTimeoutMs, pollMs))) {
    throw new Error("The exact qualification process remained alive after bounded cleanup.");
  }
  return cleanupReceipt(binding, startedAt, "forced", true);
}

async function waitForExit(binding, inspect, timeoutMs, pollMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() <= deadline) {
    const current = inspect(binding.pid);
    if (!current) return true;
    if (!sameProcessLifetime(binding, current)) {
      throw new Error("Qualification PID was reused before cleanup could be verified.");
    }
    await sleep(pollMs);
  }
  return false;
}

function cleanupReceipt(binding, startedAt, outcome, forced) {
  return {
    schemaVersion: CLEANUP_RECEIPT_SCHEMA_VERSION,
    kind: CLEANUP_RECEIPT_KIND,
    status: "passed",
    synthetic: false,
    startedAt,
    completedAt: new Date().toISOString(),
    pid: binding.pid,
    parentPid: binding.parentPid,
    processGroupId: binding.processGroupId,
    executableIdentitySha256: binding.executableIdentitySha256,
    loadedImageIdentitySha256: binding.loadedImageIdentitySha256,
    executableSha256: binding.executableSha256,
    outcome,
    forced,
    exitVerified: true,
  };
}

function valuesFor(argv) {
  const command = argv[0];
  const values = {};
  for (let index = 1; index < argv.length; index += 2) {
    const key = argv[index];
    const value = argv[index + 1];
    if (!key?.startsWith("--") || !value || value.startsWith("--")) {
      throw new Error("qualification-process requires named value arguments");
    }
    values[key.slice(2)] = value;
  }
  return { command, values };
}

async function main() {
  const { command, values } = valuesFor(process.argv.slice(2));
  if (command === "capture") {
    const binding = captureOwnedProcess(Number(values.pid), values.executable);
    atomicWriteFile(values.output, `${JSON.stringify(binding, null, 2)}\n`, 0o600);
    process.stdout.write(`OOMU_QUALIFICATION_PROCESS_BINDING=${resolve(values.output)}\n`);
    return;
  }
  if (command === "stop") {
    const binding = JSON.parse(readFileSync(resolve(values.binding), "utf8"));
    const receipt = await stopOwnedProcess(binding);
    atomicWriteFile(values.output, `${JSON.stringify(receipt, null, 2)}\n`, 0o400);
    process.stdout.write(`OOMU_QUALIFICATION_PROCESS_CLEANUP=${resolve(values.output)}\n`);
    return;
  }
  throw new Error("Usage: qualification-process.mjs <capture|stop> [options]");
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  main().catch((error) => {
    console.error(`OOMU QUALIFICATION PROCESS FAILED: ${error.message}`);
    process.exit(1);
  });
}
