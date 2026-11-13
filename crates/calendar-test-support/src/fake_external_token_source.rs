//! Port of `CalendarTestSupport/FakeExternalTokenSource.swift`.

use std::sync::{Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use calendar_core::external_tokens::{
    ExternalAccessToken, ExternalAccount, ExternalSignInRequest, ExternalSignInResult,
    ExternalTokenSource,
};
use calendar_core::failure::{Failure, Outcome};
use calendar_core::source_types::SourceError;

/// One recorded silent token request (Swift `FakeExternalTokenSource.TokenCall`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenCall {
    /// The account the token was requested for.
    pub account: ExternalAccount,
    /// The scopes requested.
    pub scopes: Vec<String>,
    /// Whether the request asked the account service to skip its cache.
    pub force_refresh: bool,
}

impl TokenCall {
    /// Creates a record of one silent token request.
    pub fn new(account: ExternalAccount, scopes: Vec<String>, force_refresh: bool) -> TokenCall {
        TokenCall {
            account,
            scopes,
            force_refresh,
        }
    }
}

#[derive(Default)]
struct State {
    token_calls: Vec<TokenCall>,
    sign_in_calls: Vec<ExternalSignInRequest>,
    token_script: Vec<Outcome<ExternalAccessToken>>,
    sign_in_script: Vec<Outcome<ExternalSignInResult>>,
}

/// A scripted `ExternalTokenSource` for tests of connectors and of hosts. Silent requests and sign-ins each consume
/// their scripted answers in order and repeat the last one; with nothing scripted they fail with
/// `SourceError::InvalidResponse`. Every call is recorded.
#[derive(Default)]
pub struct FakeExternalTokenSource {
    state: Mutex<State>,
}

impl FakeExternalTokenSource {
    /// Swift `init()`: nothing scripted.
    pub fn new() -> FakeExternalTokenSource {
        FakeExternalTokenSource::default()
    }

    /// Swift `init(tokens:signIns:)`.
    pub fn with_scripts(
        tokens: Vec<Outcome<ExternalAccessToken>>,
        sign_ins: Vec<Outcome<ExternalSignInResult>>,
    ) -> FakeExternalTokenSource {
        FakeExternalTokenSource {
            state: Mutex::new(State {
                token_script: tokens,
                sign_in_script: sign_ins,
                ..State::default()
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Replaces the scripted silent answers.
    pub fn script_tokens(&self, results: Vec<Outcome<ExternalAccessToken>>) {
        self.lock().token_script = results;
    }

    /// Replaces the scripted sign-in answers.
    pub fn script_sign_ins(&self, results: Vec<Outcome<ExternalSignInResult>>) {
        self.lock().sign_in_script = results;
    }

    /// Swift `tokenCalls`: every silent request, in order.
    pub fn token_calls(&self) -> Vec<TokenCall> {
        self.lock().token_calls.clone()
    }

    /// Swift `signInCalls`: every sign-in request, in order.
    pub fn sign_in_calls(&self) -> Vec<ExternalSignInRequest> {
        self.lock().sign_in_calls.clone()
    }
}

/// Swift `next`: the first scripted answer, removed unless it is the last one.
fn next<T: Clone>(script: &mut Vec<Outcome<T>>) -> Outcome<T> {
    let Some(first) = script.first().cloned() else {
        return Err(Failure::Source(SourceError::InvalidResponse(
            "FakeExternalTokenSource has no scripted answer".to_string(),
        )));
    };
    if script.len() > 1 {
        drop(script.remove(0));
    }
    first
}

#[async_trait]
impl ExternalTokenSource for FakeExternalTokenSource {
    async fn access_token(
        &self,
        account: &ExternalAccount,
        scopes: &[String],
        force_refresh: bool,
    ) -> Outcome<ExternalAccessToken> {
        let mut state = self.lock();
        state.token_calls.push(TokenCall::new(
            account.clone(),
            scopes.to_vec(),
            force_refresh,
        ));
        next(&mut state.token_script)
    }

    async fn sign_in(&self, request: &ExternalSignInRequest) -> Outcome<ExternalSignInResult> {
        let mut state = self.lock();
        state.sign_in_calls.push(request.clone());
        next(&mut state.sign_in_script)
    }
}
