//! Direct tests of `FakeExternalTokenSource` (no Swift counterpart; see `fake_transport_tests`).

use calendar_core::external_tokens::{
    ExternalAccessToken, ExternalAccount, ExternalSignInRequest, ExternalSignInResult,
    ExternalTokenSource,
};
use calendar_core::failure::Failure;
use calendar_core::source_types::SourceError;

use super::support::at;
use crate::fake_external_token_source::{FakeExternalTokenSource, TokenCall};

fn token(text: &str) -> ExternalAccessToken {
    ExternalAccessToken::new(text, at(1_800_000_000))
}

fn scopes() -> Vec<String> {
    vec!["calendars.read".to_string()]
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn with_nothing_scripted_both_calls_fail_with_invalid_response() {
    let source = FakeExternalTokenSource::new();
    let expected = Failure::Source(SourceError::InvalidResponse(
        "FakeExternalTokenSource has no scripted answer".to_string(),
    ));
    let account = ExternalAccount::new("acct");
    assert_eq!(
        source
            .access_token(&account, &scopes(), false)
            .await
            .unwrap_err(),
        expected
    );
    assert_eq!(
        source
            .sign_in(&ExternalSignInRequest::new(scopes()))
            .await
            .unwrap_err(),
        expected
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn silent_answers_are_consumed_in_order_and_the_last_repeats() {
    let source = FakeExternalTokenSource::with_scripts(
        vec![
            Ok(token("one")),
            Err(Failure::Source(SourceError::AuthExpired)),
            Ok(token("three")),
        ],
        vec![],
    );
    let account = ExternalAccount::new("acct");
    let mut seen = Vec::new();
    for _ in 0..5 {
        seen.push(
            source
                .access_token(&account, &scopes(), false)
                .await
                .map(|t| t.access_token),
        );
    }
    assert_eq!(
        seen,
        vec![
            Ok("one".to_string()),
            Err(Failure::Source(SourceError::AuthExpired)),
            Ok("three".to_string()),
            Ok("three".to_string()),
            Ok("three".to_string()),
        ]
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn sign_in_answers_are_consumed_in_order_and_the_last_repeats() {
    let result = |id: &str| ExternalSignInResult::new(ExternalAccount::new(id), token("t"));
    let source = FakeExternalTokenSource::with_scripts(
        vec![],
        vec![
            Ok(result("first")),
            Err(Failure::Cancelled),
            Ok(result("last")),
        ],
    );
    let request = ExternalSignInRequest::new(scopes());
    let mut seen = Vec::new();
    for _ in 0..4 {
        seen.push(source.sign_in(&request).await.map(|r| r.account.id));
    }
    assert_eq!(
        seen,
        vec![
            Ok("first".to_string()),
            Err(Failure::Cancelled),
            Ok("last".to_string()),
            Ok("last".to_string()),
        ]
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn every_call_is_recorded() {
    let source = FakeExternalTokenSource::with_scripts(vec![Ok(token("t"))], vec![]);
    let account = ExternalAccount::new("acct");
    source
        .access_token(&account, &scopes(), false)
        .await
        .unwrap();
    source
        .access_token(&account, &scopes(), true)
        .await
        .unwrap();
    let mut request = ExternalSignInRequest::new(scopes());
    request.email = Some("hint@example.test".to_string());
    let _ = source.sign_in(&request).await;

    assert_eq!(
        source.token_calls(),
        vec![
            TokenCall::new(account.clone(), scopes(), false),
            TokenCall::new(account, scopes(), true),
        ]
    );
    assert_eq!(source.sign_in_calls(), vec![request]);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn scripting_replaces_the_previous_script() {
    let source = FakeExternalTokenSource::with_scripts(vec![Ok(token("old"))], vec![]);
    let account = ExternalAccount::new("acct");
    assert_eq!(
        source
            .access_token(&account, &[], false)
            .await
            .unwrap()
            .access_token,
        "old"
    );
    source.script_tokens(vec![Ok(token("new"))]);
    assert_eq!(
        source
            .access_token(&account, &[], false)
            .await
            .unwrap()
            .access_token,
        "new"
    );
    source.script_sign_ins(vec![Err(Failure::Source(SourceError::AuthExpired))]);
    assert_eq!(
        source
            .sign_in(&ExternalSignInRequest::new(vec![]))
            .await
            .unwrap_err(),
        Failure::Source(SourceError::AuthExpired)
    );
}
