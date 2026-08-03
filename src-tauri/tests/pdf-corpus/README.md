# Deterministic PDF containment corpus

`tests/pdf_containment.rs` generates this corpus from reviewed source on every
test run and passes the resulting bytes to the real `pdf_extract_helper`
process. No parser result is mocked and no network fixture is required.

The corpus contains:

- an ordinary one-page text PDF with a known extracted sentence;
- valid scanned/image-only and image-heavy DCT PDFs with no fabricated text expectation;
- the `RUSTSEC-2026-0187` regression shape (deeply nested literal containers);
- cyclic indirect references;
- a Flate stream that expands beyond the decompression budget;
- a malformed cross-reference offset;
- a real password-encrypted document produced by lopdf;
- a 129-page tree, exceeding the reviewed page budget;
- an image object exceeding the reviewed dimension budget; and
- a direct-object collection exceeding the reviewed object budget.

Every document is deterministic. Expected outcomes are verified text, verified
empty text, a contained parser completion, or a named containment error. The evidence test emits
actual helper wall time, CPU time, peak resident memory, and triggered limit for
each process so the canonical release pipeline can preserve those measurements.
