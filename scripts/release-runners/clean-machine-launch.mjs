#!/usr/bin/env node
import { runRemoteReleaseJob } from "./remote-runner-client.mjs";
try { await runRemoteReleaseJob("clean-machine-launch", process.argv.slice(2)); }
catch (error) { console.error(`CLEAN MACHINE RELEASE TEST FAILED: ${error.message}`); process.exit(1); }
