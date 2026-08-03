use lopdf::{
    dictionary, Document, EncryptionState, EncryptionVersion, Object, Permissions, Stream,
};
use oomu_lib::pdf_containment::{
    extract_pdf_bytes_with_helper, extract_pdf_bytes_with_helper_and_cancellation,
    extract_pdf_from_open_file, PdfContainmentError,
};
use serde_json::{json, Value};
use std::{
    fs,
    io::Cursor,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

const ORDINARY_TEXT: &str = "OOMU contained PDF extraction is real.";

fn helper_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_pdf_extract_helper"))
}

fn document_with_pages(page_count: usize, text: Option<&str>) -> Document {
    let mut document = Document::with_version("1.7");
    let pages_id = document.new_object_id();
    let catalog_id = document.new_object_id();
    let font_id = document.new_object_id();
    let content_id = document.new_object_id();
    document.objects.insert(
        font_id,
        Object::Dictionary(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica"
        }),
    );
    let content = text
        .map(|value| format!("BT /F1 12 Tf 40 740 Td ({value}) Tj ET"))
        .unwrap_or_default();
    document.objects.insert(
        content_id,
        Object::Stream(Stream::new(dictionary! {}, content.into_bytes())),
    );
    let mut children = Vec::with_capacity(page_count);
    for _ in 0..page_count {
        let page_id = document.new_object_id();
        document.objects.insert(
            page_id,
            Object::Dictionary(dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
                "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
                "Resources" => dictionary! {"Font" => dictionary! {"F1" => font_id}},
                "Contents" => content_id
            }),
        );
        children.push(page_id.into());
    }
    document.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => children,
            "Count" => page_count as i64
        }),
    );
    document.objects.insert(
        catalog_id,
        Object::Dictionary(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id
        }),
    );
    document.trailer.set("Root", catalog_id);
    document.trailer.set(
        "ID",
        vec![
            Object::string_literal("oomu-pdf-corpus-document-id"),
            Object::string_literal("oomu-pdf-corpus-document-id"),
        ],
    );
    document
}

fn save_document(mut document: Document) -> Vec<u8> {
    let mut output = Cursor::new(Vec::new());
    document
        .save_to(&mut output)
        .expect("fixture must serialize");
    output.into_inner()
}

fn ordinary_pdf() -> Vec<u8> {
    save_document(document_with_pages(1, Some(ORDINARY_TEXT)))
}

fn scanned_pdf() -> Vec<u8> {
    let mut document = document_with_pages(1, None);
    let image_id = document.new_object_id();
    document.objects.insert(
        image_id,
        Object::Stream(Stream::new(
            dictionary! {
                "Type" => "XObject",
                "Subtype" => "Image",
                "Width" => 32,
                "Height" => 32,
                "ColorSpace" => "DeviceGray",
                "BitsPerComponent" => 8
            },
            vec![0_u8; 32 * 32],
        )),
    );
    save_document(document)
}

fn image_heavy_pdf() -> Vec<u8> {
    let mut document = document_with_pages(1, None);
    for _ in 0..3 {
        let image_id = document.new_object_id();
        document.objects.insert(
            image_id,
            Object::Stream(Stream::new(
                dictionary! {
                    "Type" => "XObject",
                    "Subtype" => "Image",
                    "Width" => 2_000,
                    "Height" => 2_000,
                    "ColorSpace" => "DeviceRGB",
                    "BitsPerComponent" => 8,
                    "Filter" => "DCTDecode"
                },
                b"\xff\xd8\xff\xd9".to_vec(),
            )),
        );
    }
    save_document(document)
}

fn deeply_nested_pdf() -> Vec<u8> {
    // Exact RUSTSEC-2026-0187 regression shape: a small, valid PDF with a
    // Catalog value containing roughly 10,000 nested arrays. lopdf <=0.41
    // aborts the process while parsing this shape instead of returning Err.
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let catalog_offset = bytes.len();
    bytes.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R /X ");
    bytes.extend(std::iter::repeat_n(b'[', 10_380));
    bytes.extend(std::iter::repeat_n(b']', 10_380));
    bytes.extend_from_slice(b" >>\nendobj\n");
    let pages_offset = bytes.len();
    bytes.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [] /Count 0 >>\nendobj\n");
    let xref_offset = bytes.len();
    bytes.extend_from_slice(
        format!(
            "xref\n0 3\n0000000000 65535 f \n{catalog_offset:010} 00000 n \n{pages_offset:010} 00000 n \ntrailer\n<< /Size 3 /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n"
        )
        .as_bytes(),
    );
    bytes
}

fn cyclic_reference_pdf() -> Vec<u8> {
    let mut document = document_with_pages(1, Some("cycle-safe"));
    let first = document.new_object_id();
    let second = document.new_object_id();
    document.objects.insert(first, Object::Reference(second));
    document.objects.insert(second, Object::Reference(first));
    save_document(document)
}

fn decompression_bomb_pdf() -> Vec<u8> {
    let mut document = document_with_pages(1, Some("compressed"));
    let stream_id = document.new_object_id();
    let mut stream = Stream::new(dictionary! {}, vec![b'A'; 65 * 1024 * 1024]);
    stream
        .compress()
        .expect("deterministic stream must compress");
    document.objects.insert(stream_id, Object::Stream(stream));
    save_document(document)
}

fn malformed_xref_pdf() -> Vec<u8> {
    let mut bytes = ordinary_pdf();
    if let Some(offset) = bytes
        .windows(b"startxref\n".len())
        .rposition(|window| window == b"startxref\n")
    {
        let number_start = offset + b"startxref\n".len();
        let number_end = bytes[number_start..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|relative| number_start + relative)
            .unwrap();
        bytes.splice(number_start..number_end, b"999999999".iter().copied());
    }
    bytes
}

fn encrypted_pdf() -> Vec<u8> {
    let mut document = document_with_pages(1, Some("protected"));
    let state = EncryptionState::try_from(EncryptionVersion::V2 {
        document: &document,
        owner_password: "owner-password",
        user_password: "user-password",
        key_length: 128,
        permissions: Permissions::all(),
    })
    .expect("fixture encryption state");
    document.encrypt(&state).expect("fixture encryption");
    save_document(document)
}

fn large_page_tree_pdf() -> Vec<u8> {
    save_document(document_with_pages(129, Some("page")))
}

fn image_dimension_bomb_pdf() -> Vec<u8> {
    let mut document = document_with_pages(1, None);
    let image_id = document.new_object_id();
    document.objects.insert(
        image_id,
        Object::Stream(Stream::new(
            dictionary! {
                "Type" => "XObject",
                "Subtype" => "Image",
                "Width" => 9_000,
                "Height" => 16,
                "ColorSpace" => "DeviceGray",
                "BitsPerComponent" => 8
            },
            vec![0_u8; 16],
        )),
    );
    save_document(document)
}

fn excessive_object_pdf() -> Vec<u8> {
    let mut document = document_with_pages(1, None);
    let collection_id = document.new_object_id();
    document.objects.insert(
        collection_id,
        Object::Array((0..50_100).map(|_| Object::Null).collect()),
    );
    save_document(document)
}

fn oversized_text_pdf() -> Vec<u8> {
    save_document(document_with_pages(1, Some(&"A".repeat(256 * 1024))))
}

fn error_measurement(name: &str, elapsed_ms: u64, error: PdfContainmentError) -> Value {
    json!({
        "name": name,
        "outcome": "bounded_rejection",
        "error_code": error.code,
        "limit_triggered": error.limit_triggered,
        "parent_observed_wall_time_ms": elapsed_ms,
        "helper_wall_time_ms": error.wall_time_ms,
        "helper_cpu_time_ms": error.cpu_time_ms,
        "helper_peak_memory_bytes": error.peak_memory_bytes,
    })
}

#[test]
fn ordinary_text_and_scanned_documents_use_the_real_helper() {
    let helper = helper_path();
    let ordinary = extract_pdf_bytes_with_helper(&ordinary_pdf(), &helper).unwrap();
    assert_eq!(ordinary.page_count, 1);
    assert!(ordinary.text.contains(ORDINARY_TEXT));
    assert!(!ordinary.truncated);
    assert!(ordinary.peak_memory_bytes > 0);

    let scanned = extract_pdf_bytes_with_helper(&scanned_pdf(), &helper).unwrap();
    assert_eq!(scanned.page_count, 1);
    assert!(
        scanned.text.trim().is_empty(),
        "text must never be fabricated"
    );
}

#[test]
fn real_corpus_processes_are_bounded_and_deterministic() {
    let helper = helper_path();
    let cases = vec![
        ("ordinary_text", ordinary_pdf(), "accept_text"),
        ("scanned_image_only", scanned_pdf(), "accept_empty"),
        ("image_heavy_dct", image_heavy_pdf(), "accept_empty"),
        ("advisory_nested_objects", deeply_nested_pdf(), "bounded"),
        ("cyclic_references", cyclic_reference_pdf(), "accept_cycle"),
        ("oversized_flate_stream", decompression_bomb_pdf(), "reject"),
        ("malformed_xref", malformed_xref_pdf(), "bounded"),
        ("encrypted_document", encrypted_pdf(), "reject"),
        ("large_page_tree", large_page_tree_pdf(), "reject"),
        ("image_dimension_bomb", image_dimension_bomb_pdf(), "reject"),
        ("excessive_direct_objects", excessive_object_pdf(), "reject"),
        (
            "oversized_text_output",
            oversized_text_pdf(),
            "accept_truncated",
        ),
    ];
    let mut measurements = Vec::new();
    for (name, bytes, expectation) in cases {
        let started = Instant::now();
        let result = extract_pdf_bytes_with_helper(&bytes, &helper);
        let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        assert!(
            elapsed_ms <= 6_000,
            "{name} exceeded the parent wall budget"
        );
        match (expectation, result) {
            ("accept_text", Ok(extraction)) => {
                assert!(extraction.text.contains(ORDINARY_TEXT));
                measurements.push(json!({
                    "name": name,
                    "outcome": "accepted",
                    "parent_observed_wall_time_ms": elapsed_ms,
                    "helper_wall_time_ms": extraction.wall_time_ms,
                    "helper_cpu_time_ms": extraction.cpu_time_ms,
                    "helper_peak_memory_bytes": extraction.peak_memory_bytes,
                    "object_count": extraction.object_count,
                    "decompressed_bytes": extraction.decompressed_bytes,
                }));
            }
            ("accept_empty", Ok(extraction)) => {
                assert!(extraction.text.trim().is_empty());
                measurements.push(json!({
                    "name": name,
                    "outcome": "accepted_empty",
                    "parent_observed_wall_time_ms": elapsed_ms,
                    "helper_wall_time_ms": extraction.wall_time_ms,
                    "helper_cpu_time_ms": extraction.cpu_time_ms,
                    "helper_peak_memory_bytes": extraction.peak_memory_bytes,
                }));
            }
            ("accept_cycle", Ok(extraction)) => {
                assert!(extraction.text.contains("cycle-safe"));
                measurements.push(json!({
                    "name": name,
                    "outcome": "accepted_cycle_without_recursion",
                    "parent_observed_wall_time_ms": elapsed_ms,
                    "helper_wall_time_ms": extraction.wall_time_ms,
                    "helper_cpu_time_ms": extraction.cpu_time_ms,
                    "helper_peak_memory_bytes": extraction.peak_memory_bytes,
                }));
            }
            ("accept_truncated", Ok(extraction)) => {
                assert!(extraction.truncated);
                assert_eq!(extraction.text.len(), 128 * 1024);
                measurements.push(json!({
                    "name": name,
                    "outcome": "accepted_at_output_limit",
                    "parent_observed_wall_time_ms": elapsed_ms,
                    "helper_wall_time_ms": extraction.wall_time_ms,
                    "helper_cpu_time_ms": extraction.cpu_time_ms,
                    "helper_peak_memory_bytes": extraction.peak_memory_bytes,
                    "output_text_bytes": extraction.text.len(),
                }));
            }
            ("reject", Err(error)) => {
                assert_ne!(error.code, "pdf_helper_protocol_failed", "{name}");
                measurements.push(error_measurement(name, elapsed_ms, error));
            }
            ("bounded", Ok(extraction)) => measurements.push(json!({
                "name": name,
                "outcome": "parser_completed_within_containment",
                "parent_observed_wall_time_ms": elapsed_ms,
                "helper_wall_time_ms": extraction.wall_time_ms,
                "helper_cpu_time_ms": extraction.cpu_time_ms,
                "helper_peak_memory_bytes": extraction.peak_memory_bytes,
            })),
            ("bounded", Err(error)) => {
                measurements.push(error_measurement(name, elapsed_ms, error));
            }
            (_, Ok(_)) => panic!("{name} unexpectedly succeeded"),
            (_, Err(error)) => panic!("{name} unexpectedly failed: {}", error.code),
        }
    }
    println!(
        "PDF_CONTAINMENT_EVIDENCE_JSON={}",
        json!({
            "parser": {"name": "lopdf", "version": "0.42.0"},
            "advisory": {"id": "RUSTSEC-2026-0187", "affected_version_present": false},
            "helper_protocol_version": 1,
            "corpus_origin": "deterministically generated from src-tauri/tests/pdf_containment.rs",
            "measurements": measurements,
        })
    );
}

#[test]
fn malformed_and_oversized_inputs_never_create_false_success() {
    let helper = helper_path();
    let malformed = extract_pdf_bytes_with_helper(b"%PDF-not-a-document", &helper).unwrap_err();
    assert!(matches!(
        malformed.code,
        "malformed_document" | "pdf_helper_terminated"
    ));

    let oversized = vec![0_u8; 8 * 1024 * 1024 + 1];
    let error = extract_pdf_bytes_with_helper(&oversized, &helper).unwrap_err();
    assert_eq!(error.code, "pdf_input_limit_exceeded");
}

#[test]
fn cancellation_kills_and_reaps_the_real_helper() {
    let helper = helper_path();
    let input = decompression_bomb_pdf();
    let cancelled = Arc::new(AtomicBool::new(false));
    let cancel_worker = Arc::clone(&cancelled);
    let trigger = thread::spawn(move || {
        thread::sleep(Duration::from_millis(20));
        cancel_worker.store(true, Ordering::Release);
    });
    let started = Instant::now();
    let error = extract_pdf_bytes_with_helper_and_cancellation(&input, &helper, &cancelled)
        .expect_err("cancellation must never return partial success");
    trigger.join().unwrap();
    assert_eq!(error.code, "pdf_helper_cancelled");
    assert!(started.elapsed() < Duration::from_secs(2));
    // The API returns only after ChildGuard has waited for the terminated PID;
    // no detached helper remains for the test process to reap.
}

#[test]
fn already_opened_picker_handle_wins_over_a_replaced_path() {
    let root = std::env::temp_dir().join(format!(
        "oomu-pdf-handle-test-{}-{}",
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("selected.pdf");
    fs::write(&path, ordinary_pdf()).unwrap();
    let approved_handle = fs::File::open(&path).unwrap();
    fs::rename(&path, root.join("approved-original.pdf")).unwrap();
    fs::write(&path, b"%PDF-malicious-replacement").unwrap();

    let extraction = extract_pdf_from_open_file(approved_handle).unwrap();
    assert!(extraction.text.contains(ORDINARY_TEXT));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn protocol_violation_is_scoped_and_never_returns_content() {
    let error =
        extract_pdf_bytes_with_helper(&ordinary_pdf(), PathBuf::from("/bin/echo").as_path())
            .expect_err("an unrelated executable cannot impersonate the typed helper protocol");
    assert_eq!(error.code, "pdf_helper_protocol_failed");
    assert!(error.message.contains("No document content was accepted"));
}
