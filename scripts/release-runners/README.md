# Release lab runner contract

The three repository entry points in this directory submit the exact candidate bytes to the
Eldris release lab over mutual TLS 1.3. They never report success locally and never substitute a
different artifact.

The release lab exposes `POST /v1/release-jobs/{job}` for `clean-machine-launch` and
`p0-acceptance`. The request body is the candidate artifact. The bounded
`X-OOMU-Release-Request` header binds its SHA-256, build metadata, and requested test. A passing
response must bind the same SHA-256, declare `synthetic: false`, and return the measured evidence
required by the canonical release pipeline.

The clean-machine service must run the real candidate on current patched macOS 14, macOS 15, and
the current supported macOS release. Every row probes voice capture, Vision OCR, PDF creation and
rendering, local inference, and PDF extraction. Before it can report success, the service must
also verify the installed application against the signed manifest's exact application-subtree
digest, strictly verify every nested code object, semantically verify the signed entitlements,
capture the production `runtime_identity` and `single_instance` native receipts, and stop the
exact test-owned process with a passing `exact_process_cleanup` receipt. The request supplies the
signed manifest payload hash, application prefix, subtree digest, and entry count. Missing real
evidence is an environment-not-ready failure, never a pass. The P0 service returns the named JSON
evidence files consumed by `p0-release-acceptance.mjs`.

The installed-app verifier must use `inspectSignedCandidateAndEvidence` from
`scripts/release-candidate-integrity.mjs` and return its exact object as
`release_candidate_evidence`. The stable fields are `bundleIdentifier`, `channel`, `buildNumber`,
`appTreeSha256`, `manifestSha256`, `codesignVerified`, `gatekeeperAccepted`,
`notarizationAccepted`, `nestedExecutablesVerified`, `installedTreeMatches`, `teamId`,
`designatedRequirementSha256`, `beforeQualificationSha256`, and
`afterQualificationSha256`. The two qualification digests must equal the signed application-tree
digest. An older runner that omits any field is not ready and must fail closed.

Required local release inputs are documented in `scripts/sign_env.sh.example`. If the service,
client identity, evidence, or exact artifact binding
is unavailable, the release fails. There is no local pass-through or simulated fallback.
