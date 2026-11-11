//! Port of `CalDAVAccountConfig.swift`: what sign-in learned about a CalDAV account, kept in `Connection.config`
//! (never a password).

use std::collections::BTreeMap;

use calendar_core::failure::{Failure, Outcome};
use calendar_core::source_types::SourceError;
use calendar_core::url_string::UrlString;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CalDavAccountConfig {
    pub server_url: UrlString,
    pub username: String,
    pub principal_url: UrlString,
    pub home_url: UrlString,
    /// The principal's `calendar-user-address-set`, lowercased: how the account appears as organizer or attendee.
    pub user_addresses: Vec<String>,
    /// The server schedules invitations itself (`calendar-auto-schedule`, RFC 6638).
    pub auto_schedule: bool,
}

fn incomplete() -> Failure {
    Failure::Source(SourceError::InvalidResponse(
        "the account settings are incomplete; sign in again".to_string(),
    ))
}

impl CalDavAccountConfig {
    pub(crate) fn new(
        server_url: UrlString,
        username: &str,
        principal_url: UrlString,
        home_url: UrlString,
        user_addresses: Vec<String>,
        auto_schedule: bool,
    ) -> CalDavAccountConfig {
        CalDavAccountConfig {
            server_url,
            username: username.to_string(),
            principal_url,
            home_url,
            user_addresses,
            auto_schedule,
        }
    }

    /// Swift `init(config:)`.
    pub(crate) fn from_config(config: &BTreeMap<String, String>) -> Outcome<CalDavAccountConfig> {
        let url = |key: &str| -> Outcome<UrlString> {
            config
                .get(key)
                .and_then(|text| UrlString::parse(text))
                .ok_or_else(incomplete)
        };
        let username = config.get("username").ok_or_else(incomplete)?.clone();
        Ok(CalDavAccountConfig {
            server_url: url("serverURL")?,
            username,
            principal_url: url("principalURL")?,
            home_url: url("homeURL")?,
            user_addresses: config
                .get("userAddresses")
                .map(String::as_str)
                .unwrap_or("")
                .split('\n')
                .filter(|a| !a.is_empty())
                .map(str::to_string)
                .collect(),
            auto_schedule: config.get("autoSchedule").map(String::as_str) == Some("true"),
        })
    }

    pub(crate) fn config(&self) -> BTreeMap<String, String> {
        BTreeMap::from([
            (
                "serverURL".to_string(),
                self.server_url.as_str().to_string(),
            ),
            ("username".to_string(), self.username.clone()),
            (
                "principalURL".to_string(),
                self.principal_url.as_str().to_string(),
            ),
            ("homeURL".to_string(), self.home_url.as_str().to_string()),
            ("userAddresses".to_string(), self.user_addresses.join("\n")),
            (
                "autoSchedule".to_string(),
                if self.auto_schedule { "true" } else { "false" }.to_string(),
            ),
        ])
    }
}
