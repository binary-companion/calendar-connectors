use std::collections::BTreeMap;
use std::sync::Arc;

use calendar_core::failure::Failure;
use calendar_core::http::HttpRequest;
use calendar_core::source_types::SourceError;
use calendar_test_support::fake_transport::{FakeTransport, TestNow, json_response};
use serde_json::json;

use super::url;
use crate::oauth::{
    AuthorizationCodeError, AuthorizationError, OAuthClient, OAuthConfig, OAuthTokens,
};

fn config() -> OAuthConfig {
    OAuthConfig {
        client_secret: Some("shh".to_string()),
        extra_auth_params: BTreeMap::from([
            ("access_type".to_string(), "offline".to_string()),
            ("prompt".to_string(), "consent".to_string()),
        ]),
        ..OAuthConfig::new(
            url("https://accounts.example.com/auth"),
            url("https://oauth.example.com/token"),
            "client-1",
            vec!["scope.a".to_string(), "scope.b".to_string()],
        )
    }
}

fn client(transport: &Arc<FakeTransport>, now: &TestNow) -> OAuthClient {
    OAuthClient::new(config(), transport.clone(), now.provider())
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(v) = text
                .get(i + 1..i + 3)
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap()
}

fn body_text(request: &HttpRequest) -> String {
    String::from_utf8(request.body.clone().unwrap_or_default()).unwrap()
}

fn form(request: &HttpRequest) -> BTreeMap<String, String> {
    let body = body_text(request);
    let mut out = BTreeMap::new();
    for pair in body.split('&').filter(|p| !p.is_empty()) {
        let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
        out.insert(percent_decode(name), percent_decode(value));
    }
    out
}

fn the_client() -> (Arc<FakeTransport>, TestNow, OAuthClient) {
    let transport = Arc::new(FakeTransport::new());
    let now = TestNow::default();
    let c = client(&transport, &now);
    (transport, now, c)
}

#[test]
fn authorization_url_carries_pkce_scopes_and_extras() {
    let (_, _, c) = the_client();
    let u = c.authorization_url(&url("http://127.0.0.1:5000"), "st", "ch");
    let items: BTreeMap<String, String> = u
        .query_items()
        .into_iter()
        .map(|(name, value)| (name, value.unwrap_or_default()))
        .collect();
    assert_eq!(u.host().as_deref(), Some("accounts.example.com"));
    assert_eq!(items["client_id"], "client-1");
    assert_eq!(items["redirect_uri"], "http://127.0.0.1:5000");
    assert_eq!(items["response_type"], "code");
    assert_eq!(items["scope"], "scope.a scope.b");
    assert_eq!(items["state"], "st");
    assert_eq!(items["code_challenge"], "ch");
    assert_eq!(items["code_challenge_method"], "S256");
    assert_eq!(items["access_type"], "offline");
    assert_eq!(items["prompt"], "consent");
}

/// Pinned by running `URLComponents.queryItems` on macOS: the query allowed set minus `& = < > " # % [ \ ] ^ \` { | }`
/// and DEL, space as %20, `+` and `;` left alone, existing query replaced and fragment kept.
#[test]
fn authorization_url_encodes_values_as_url_components_does() {
    let transport = Arc::new(FakeTransport::new());
    let now = TestNow::default();
    let config = OAuthConfig::new(
        url("https://a.example/auth?old=1#frag"),
        url("https://a.example/token"),
        "c",
        vec!["a b".to_string(), "c+d".to_string()],
    );
    let c = OAuthClient::new(config, transport, now.provider());
    let u = c.authorization_url(&url("http://127.0.0.1:5000/cb?x=1&y=é"), "s&t=1", "ch");
    assert_eq!(
        u.as_str(),
        "https://a.example/auth?client_id=c&redirect_uri=http://127.0.0.1:5000/cb?x%3D1%26y%3D%C3%A9\
         &response_type=code&scope=a%20b%20c+d&state=s%26t%3D1&code_challenge=ch&code_challenge_method=S256#frag"
    );
}

#[test]
fn authorization_code_validates_state_and_errors() {
    let (_, _, c) = the_client();
    assert_eq!(
        c.authorization_code(&url("http://127.0.0.1:5000/?code=abc&state=st"), "st"),
        Ok("abc".to_string())
    );
    assert_eq!(
        c.authorization_code(&url("http://127.0.0.1:5000/?code=abc&state=other"), "st"),
        Err(AuthorizationCodeError::Authorization(
            AuthorizationError::StateMismatch
        ))
    );
    assert_eq!(
        c.authorization_code(
            &url("http://127.0.0.1:5000/?error=access_denied&state=st"),
            "st"
        ),
        Err(AuthorizationCodeError::Authorization(
            AuthorizationError::Cancelled
        ))
    );
    assert_eq!(
        c.authorization_code(&url("http://127.0.0.1:5000/?state=st"), "st"),
        Err(AuthorizationCodeError::Authorization(
            AuthorizationError::MissingCode
        ))
    );
    assert_eq!(
        c.authorization_code(
            &url("http://127.0.0.1:5000/?error=server_error&state=st"),
            "st"
        ),
        Err(AuthorizationCodeError::Source(
            SourceError::InvalidResponse("oauth error: server_error".to_string())
        ))
    );
    // An empty code is a missing code.
    assert_eq!(
        c.authorization_code(&url("http://127.0.0.1:5000/?code=&state=st"), "st"),
        Err(AuthorizationCodeError::Authorization(
            AuthorizationError::MissingCode
        ))
    );
}

#[test]
fn authorization_code_validates_state_before_error_handling() {
    let (_, _, c) = the_client();
    // Forged state with access_denied error should throw stateMismatch, not cancelled
    assert_eq!(
        c.authorization_code(
            &url("http://127.0.0.1:5000/?error=access_denied&state=forged"),
            "st"
        ),
        Err(AuthorizationCodeError::Authorization(
            AuthorizationError::StateMismatch
        ))
    );
}

#[test]
fn authorization_code_validates_state_before_missing_state() {
    let (_, _, c) = the_client();
    // Missing state with access_denied error should throw stateMismatch, not cancelled
    assert_eq!(
        c.authorization_code(&url("http://127.0.0.1:5000/?error=access_denied"), "st"),
        Err(AuthorizationCodeError::Authorization(
            AuthorizationError::StateMismatch
        ))
    );
}

/// The conversion a connector's `authorize` relies on (Swift lets both error types propagate).
#[test]
fn authorization_code_errors_convert_to_failures() {
    assert_eq!(
        Failure::from(AuthorizationCodeError::Authorization(
            AuthorizationError::Cancelled
        )),
        Failure::Cancelled
    );
    assert_eq!(
        Failure::from(AuthorizationCodeError::Source(SourceError::AuthExpired)),
        Failure::Source(SourceError::AuthExpired)
    );
    assert!(matches!(
        Failure::from(AuthorizationCodeError::Authorization(
            AuthorizationError::StateMismatch
        )),
        Failure::Host(_)
    ));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn exchange_posts_form_and_parses_tokens() {
    let (transport, now, c) = the_client();
    transport.route(
        "oauth.example.com/token",
        vec![json_response(
            &json!({"access_token": "at", "expires_in": 3600, "refresh_token": "rt"}),
            200,
        )],
    );
    let tokens = c
        .exchange("the+code/1", "ver", &url("http://127.0.0.1:5000"))
        .await
        .unwrap();
    assert_eq!(tokens.access_token, "at");
    assert_eq!(tokens.refresh_token.as_deref(), Some("rt"));
    assert_eq!(
        tokens.expires_at,
        calendar_core::time::add_seconds(now.date(), 3600.0)
    );
    let request = transport.requests().first().cloned().unwrap();
    assert_eq!(request.method, "POST");
    assert_eq!(request.url.as_str(), "https://oauth.example.com/token");
    assert_eq!(
        request.headers.get("Content-Type").map(String::as_str),
        Some("application/x-www-form-urlencoded")
    );
    assert_eq!(
        request.headers.get("Accept").map(String::as_str),
        Some("application/json")
    );
    let f = form(&request);
    assert_eq!(f["grant_type"], "authorization_code");
    assert_eq!(f["code"], "the+code/1");
    assert_eq!(f["code_verifier"], "ver");
    assert_eq!(f["client_id"], "client-1");
    assert_eq!(f["client_secret"], "shh");
    assert_eq!(f["redirect_uri"], "http://127.0.0.1:5000");
    // Verify raw body contains properly percent-encoded values
    let raw = body_text(&request);
    assert!(raw.contains("code=the%2Bcode%2F1"));
    assert!(raw.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A5000"));
    // Fields are sorted by name.
    let names: Vec<&str> = raw
        .split('&')
        .map(|p| p.split('=').next().unwrap())
        .collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    assert_eq!(names, sorted);
}

/// Swift leaves non-ASCII letters raw in the body (`CharacterSet.alphanumerics`); the Rust port percent-encodes
/// every non-ASCII character, which a form parser reads back identically.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn exchange_percent_encodes_everything_but_unreserved_characters() {
    let (transport, _, c) = the_client();
    transport.route(
        "token",
        vec![json_response(&json!({"access_token": "at"}), 200)],
    );
    c.exchange("a b~c-d.e_f\u{e9}&=", "v", &url("http://127.0.0.1:1"))
        .await
        .unwrap();
    let raw = body_text(&transport.requests()[0]);
    assert!(raw.contains("code=a%20b~c-d.e_f%C3%A9%26%3D"), "{raw}");
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_body_has_no_client_secret_when_there_is_none() {
    let transport = Arc::new(FakeTransport::new());
    transport.route(
        "token",
        vec![json_response(&json!({"access_token": "at"}), 200)],
    );
    let now = TestNow::default();
    let c = OAuthClient::new(
        OAuthConfig::new(
            url("https://a.example/auth"),
            url("https://a.example/token"),
            "c",
            vec![],
        ),
        transport.clone(),
        now.provider(),
    );
    c.refresh("rt").await.unwrap();
    assert!(!form(&transport.requests()[0]).contains_key("client_secret"));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn refresh_maps_invalid_grant_to_auth_expired() {
    let (transport, _, c) = the_client();
    transport.route(
        "token",
        vec![json_response(&json!({"error": "invalid_grant"}), 400)],
    );
    assert_eq!(
        c.refresh("old").await,
        Err(Failure::Source(SourceError::AuthExpired))
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn refresh_sends_grant_and_keeps_nil_refresh_token() {
    let (transport, _, c) = the_client();
    transport.route(
        "token",
        vec![json_response(
            &json!({"access_token": "at2", "expires_in": 100}),
            200,
        )],
    );
    let tokens = c.refresh("rt").await.unwrap();
    assert_eq!(tokens.refresh_token, None);
    let f = form(&transport.requests()[0]);
    assert_eq!(f["grant_type"], "refresh_token");
    assert_eq!(f["refresh_token"], "rt");
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn exchange_other_oauth_errors_are_invalid_response() {
    let (transport, _, c) = the_client();
    transport.route(
        "token",
        vec![json_response(&json!({"error": "invalid_grant"}), 400)],
    );
    assert_eq!(
        c.exchange("c", "v", &url("http://127.0.0.1:1")).await,
        Err(Failure::Source(SourceError::InvalidResponse(
            "oauth error: invalid_grant".to_string()
        )))
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn refresh_maps_interaction_required_to_auth_expired() {
    let transport = Arc::new(FakeTransport::new());
    transport.route(
        "token",
        vec![json_response(
            &json!({"error": "interaction_required"}),
            400,
        )],
    );
    let now = TestNow::default();
    let c = OAuthClient::new(
        OAuthConfig::new(
            url("https://a.example/auth"),
            url("https://a.example/token"),
            "c",
            vec![],
        ),
        transport,
        now.provider(),
    );
    assert_eq!(
        c.refresh("rt").await,
        Err(Failure::Source(SourceError::AuthExpired))
    );
}

/// Not in the Swift tests, but pinned by the Swift code: a failure without an `error` field is named by its status,
/// an access token is required even on 200, a missing `expires_in` means 3600 s, and a transport failure passes
/// through.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn token_response_edge_cases_follow_the_swift_decoder() {
    let (transport, now, c) = the_client();
    transport.route(
        "token",
        vec![calendar_test_support::fake_transport::text_response(
            "<html>", 502,
        )],
    );
    assert_eq!(
        c.refresh("rt").await,
        Err(Failure::Source(SourceError::InvalidResponse(
            "oauth error: HTTP 502".to_string()
        )))
    );
    transport.route(
        "token",
        vec![json_response(&json!({"refresh_token": "x"}), 200)],
    );
    assert_eq!(
        c.refresh("rt").await,
        Err(Failure::Source(SourceError::InvalidResponse(
            "oauth error: HTTP 200".to_string()
        )))
    );
    // A 4xx with an access token is still a failure.
    transport.route(
        "token",
        vec![json_response(
            &json!({"access_token": "a", "error": "x"}),
            400,
        )],
    );
    assert_eq!(
        c.refresh("rt").await,
        Err(Failure::Source(SourceError::InvalidResponse(
            "oauth error: x".to_string()
        )))
    );
    // Unknown fields, nulls and a missing lifetime.
    transport.route(
        "token",
        vec![json_response(
            &json!({"access_token": "a", "token_type": "Bearer", "scope": null, "expires_in": null, "extra": {"k": [1, 2]}}),
            200,
        )],
    );
    assert_eq!(
        c.refresh("rt").await.unwrap(),
        OAuthTokens::new("a", calendar_core::time::add_seconds(now.date(), 3600.0))
    );
    // `refresh` of an `interaction_required` on exchange is not mapped (only refreshes expire the auth).
    transport.route(
        "token",
        vec![json_response(
            &json!({"error": "interaction_required"}),
            400,
        )],
    );
    assert_eq!(
        c.exchange("c", "v", &url("http://127.0.0.1:1")).await,
        Err(Failure::Source(SourceError::InvalidResponse(
            "oauth error: interaction_required".to_string()
        )))
    );
    // A transport failure passes through untouched.
    let failing = Arc::new(FakeTransport::new());
    failing.route_outcomes(
        "token",
        vec![Err(Failure::Source(SourceError::Network(
            "offline".to_string(),
        )))],
    );
    let c2 = OAuthClient::new(config(), failing, now.provider());
    assert_eq!(
        c2.refresh("rt").await,
        Err(Failure::Source(SourceError::Network("offline".to_string())))
    );
}

/// The token endpoint's error text reaches `InvalidResponse` only through the `error` field, as in Swift; the rest
/// of the body (which can carry account data in `error_description`) never does.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_error_description_never_reaches_the_failure() {
    let (transport, _, c) = the_client();
    transport.route(
        "token",
        vec![json_response(
            &json!({"error": "invalid_request", "error_description": "AADSTS70000 for secret@example.com"}),
            400,
        )],
    );
    let err = c.refresh("rt").await.unwrap_err();
    let text = format!("{err:?} {err}");
    assert!(
        !text.contains("secret@example.com") && !text.contains("AADSTS"),
        "{text}"
    );
}

#[test]
fn debug_output_hides_tokens_and_the_client_secret() {
    let tokens = OAuthTokens {
        refresh_token: Some("rt-secret".to_string()),
        ..OAuthTokens::new("at-secret", TestNow::default().date())
    };
    let text = format!("{tokens:?} {:?}", config());
    assert!(
        !text.contains("at-secret") && !text.contains("rt-secret") && !text.contains("shh"),
        "{text}"
    );
}
