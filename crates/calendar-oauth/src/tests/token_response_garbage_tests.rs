//! Review Focus 2: the token-response decoder returns a result for any bytes. It never panics and never hangs.

use std::sync::Arc;

use calendar_core::failure::Failure;
use calendar_core::http::HttpResponse;
use calendar_core::source_types::SourceError;
use calendar_test_support::fake_transport::{FakeTransport, TestNow};

use super::url;
use crate::oauth::{OAuthClient, OAuthConfig, parse_token_response};

/// A fixed-seed linear congruential generator (conventions section 9).
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }

    fn bytes(&mut self, length: usize) -> Vec<u8> {
        (0..length).map(|_| self.next() as u8).collect()
    }
}

const SAMPLE: &str = r#"{"access_token":"ya29.sample-token","expires_in":3599,"refresh_token":"1//sample-refresh","scope":"https://www.googleapis.com/auth/calendar.readonly","token_type":"Bearer","id_token":"eyJ.sample.sig"}"#;

#[test]
fn the_decoder_reads_the_sample() {
    let parsed = parse_token_response(SAMPLE.as_bytes()).unwrap();
    assert_eq!(parsed.access_token.as_deref(), Some("ya29.sample-token"));
    assert_eq!(parsed.expires_in, Some(3599.0));
    assert_eq!(parsed.refresh_token.as_deref(), Some("1//sample-refresh"));
    assert_eq!(parsed.error, None);
}

#[test]
fn the_decoder_survives_random_bytes_and_truncated_samples() {
    let mut rng = Lcg(0x5EED_0A07);
    for i in 0..2000 {
        let length = (rng.next() % 300) as usize;
        let bytes = rng.bytes(length);
        let _ = parse_token_response(&bytes);
        // Random bytes that start like JSON reach deeper into the parser.
        let mut shaped = br#"{"access_token":""#.to_vec();
        shaped.extend(rng.bytes(i % 40));
        let _ = parse_token_response(&shaped);
    }
    let sample = SAMPLE.as_bytes();
    for cut in 0..200 {
        let end = cut * sample.len() / 200;
        let parsed = parse_token_response(&sample[..end]);
        // Only the complete sample decodes.
        assert!(
            parsed.is_none() || end == sample.len(),
            "cut at {end} decoded"
        );
    }
    // Invalid UTF-8 inside a string, deep nesting and an empty body.
    assert!(parse_token_response(b"{\"access_token\":\"\xff\xfe\"}").is_none());
    assert!(parse_token_response(&[b'['; 100_000]).is_none());
    assert!(parse_token_response(b"").is_none());
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_client_turns_any_body_into_a_token_or_a_source_error() {
    let mut rng = Lcg(0xC0FF_EE11);
    let now = TestNow::default();
    for i in 0..300 {
        let transport = Arc::new(FakeTransport::new());
        let length = (rng.next() % 200) as usize;
        let body = if i % 3 == 0 {
            SAMPLE.as_bytes()[..(length % SAMPLE.len())].to_vec()
        } else {
            rng.bytes(length)
        };
        let status = [200, 400, 401, 500, 502][i % 5];
        transport.route(
            "token",
            vec![HttpResponse::new(status, Default::default(), body)],
        );
        let client = OAuthClient::new(
            OAuthConfig::new(
                url("https://a.example/auth"),
                url("https://a.example/token"),
                "c",
                vec![],
            ),
            transport,
            now.provider(),
        );
        match client.refresh("rt").await {
            Ok(_) => {}
            Err(Failure::Source(SourceError::AuthExpired | SourceError::InvalidResponse(_))) => {}
            Err(other) => panic!("unexpected failure {other:?}"),
        }
    }
}
