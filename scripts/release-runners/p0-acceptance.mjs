#!/usr/bin/env node
import { runRemoteReleaseJob } from "./remote-runner-client.mjs";
try { await runRemoteReleaseJob("p0-acceptance", process.argv.slice(2)); }
catch (error) { console.error(`P0 RELEASE TEST FAILED: ${error.message}`); process.exit(1); }
