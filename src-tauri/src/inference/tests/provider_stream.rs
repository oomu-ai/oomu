use super::*;

#[test]
fn provider_sse_decoder_preserves_unicode_split_across_network_chunks() {
    let mut decoder = SseEventDecoder::default();
    let event = "data: {\"text\":\"Café 你好\"}\r\n\r\n".as_bytes();
    let split = event
        .windows(3)
        .position(|window| window == "你".as_bytes())
        .expect("Unicode marker exists")
        + 1;

    assert!(decoder.push_chunk(&event[..split]).unwrap().is_empty());
    assert_eq!(
        decoder.push_chunk(&event[split..]).unwrap(),
        vec!["{\"text\":\"Café 你好\"}".to_string()]
    );
    assert!(decoder.finish().unwrap().is_empty());
}

#[test]
fn provider_stream_buffers_are_bounded_before_untrusted_growth() {
    let mut decoder = SseEventDecoder::default();
    let oversized_event = vec![b'a'; MAX_PROVIDER_SSE_PENDING_EVENT_BYTES + 1];
    let error = decoder.push_chunk(&oversized_event).unwrap_err();
    assert!(error.contains("SSE event"));
    assert!(error.contains("1 MiB"));

    let mut decoder = SseEventDecoder::default();
    let mut oversized_complete_event = Vec::with_capacity(MAX_PROVIDER_SSE_PENDING_EVENT_BYTES + 2);
    oversized_complete_event.extend_from_slice(b"data: ");
    oversized_complete_event.resize(MAX_PROVIDER_SSE_PENDING_EVENT_BYTES, b'a');
    oversized_complete_event.extend_from_slice(b"\n\n");
    let error = decoder.push_chunk(&oversized_complete_event).unwrap_err();
    assert!(error.contains("SSE event"));
    assert!(error.contains("1 MiB"));

    assert!(
        ensure_provider_response_text_capacity(MAX_PROVIDER_RESPONSE_TEXT_BYTES - 1, 1).is_ok()
    );
    let error =
        ensure_provider_response_text_capacity(MAX_PROVIDER_RESPONSE_TEXT_BYTES, 1).unwrap_err();
    assert_eq!(error.code, "provider_response_error");
    assert!(error.message.contains("8 MiB"));
}

fn spawn_chunked_provider_stream(
    writes: Vec<(Duration, &'static [u8])>,
    finish_body: bool,
) -> (String, JoinHandle<()>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or_default() == 0 || line == "\r\n" {
                break;
            }
        }
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
            )
            .unwrap();
        stream.flush().unwrap();
        for (delay, payload) in writes {
            thread::sleep(delay);
            let chunk_header = format!("{:X}\r\n", payload.len());
            if stream.write_all(chunk_header.as_bytes()).is_err()
                || stream.write_all(payload).is_err()
                || stream.write_all(b"\r\n").is_err()
                || stream.flush().is_err()
            {
                return;
            }
        }
        if finish_body {
            let _ = stream.write_all(b"0\r\n\r\n");
            let _ = stream.flush();
        }
    });
    (format!("http://{address}/stream"), server)
}

#[tokio::test]
async fn provider_stream_timeout_policy_allows_progress_past_the_idle_window() {
    let policy = ProviderStreamTimeoutPolicy {
        connect_timeout: Duration::from_secs(1),
        // Keep a wide scheduler margin so the real-socket assertion remains
        // deterministic while the full Rust suite is saturating the host.
        read_idle_timeout: Duration::from_secs(1),
    };
    let writes: Vec<(Duration, &'static [u8])> = (0..24)
        .map(|_| (Duration::from_millis(50), &b"data: {\"ok\":true}\n\n"[..]))
        .collect();
    let (url, server) = spawn_chunked_provider_stream(writes, true);
    let client = apply_provider_stream_timeout_policy(AsyncClient::builder(), policy)
        .build()
        .unwrap();

    let started = Instant::now();
    let response = client.get(url).send().await.unwrap();
    let body = response.bytes().await.unwrap();

    assert!(started.elapsed() >= Duration::from_millis(1_100));
    assert_eq!(
        body.windows(b"data:".len())
            .filter(|window| *window == b"data:")
            .count(),
        24
    );
    server.join().unwrap();
}

#[tokio::test]
async fn provider_stream_timeout_policy_stops_a_genuinely_idle_body() {
    let policy = ProviderStreamTimeoutPolicy {
        connect_timeout: Duration::from_secs(1),
        read_idle_timeout: Duration::from_millis(75),
    };
    let (url, server) = spawn_chunked_provider_stream(
        vec![
            (Duration::ZERO, b"data: first\n\n"),
            (Duration::from_millis(250), b"data: late\n\n"),
        ],
        true,
    );
    let client = apply_provider_stream_timeout_policy(AsyncClient::builder(), policy)
        .build()
        .unwrap();
    let response = client.get(url).send().await.unwrap();
    let mut body = response.bytes_stream();

    assert_eq!(
        body.next().await.unwrap().unwrap().as_ref(),
        b"data: first\n\n"
    );
    let error = body.next().await.unwrap().unwrap_err();

    assert!(error.is_timeout());
    server.join().unwrap();
}
