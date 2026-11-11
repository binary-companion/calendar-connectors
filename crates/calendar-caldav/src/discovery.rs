//! Port of `CalDAVDiscovery.swift`: RFC 6764 and RFC 4791 discovery.

use std::collections::BTreeMap;

use calendar_core::failure::{Failure, Outcome};
use calendar_core::source_types::SourceError;
use calendar_core::url_string::UrlString;

use crate::account_config::CalDavAccountConfig;
use crate::dav_url::DavUrl;
use crate::dav_xml::{Dav, DavProperty, DavXml};
use crate::webdav_client::WebDavClient;

fn invalid(message: impl Into<String>) -> Failure {
    Failure::Source(SourceError::InvalidResponse(message.into()))
}

/// `/.well-known/caldav` (or the server URL itself), the current user's principal, its calendar home and addresses,
/// and whether the server schedules invitations.
pub(crate) struct CalDavDiscovery {
    pub client: WebDavClient,
}

impl CalDavDiscovery {
    pub(crate) async fn discover(
        &self,
        server_url: &UrlString,
        username: &str,
    ) -> Outcome<CalDavAccountConfig> {
        let principal = self.principal_url(server_url).await?;
        let (status, answered_by) = self
            .client
            .propfind(
                &principal,
                0,
                &[
                    DavProperty::calendar_home_set(),
                    DavProperty::calendar_user_address_set(),
                ],
            )
            .await?;
        let empty = BTreeMap::new();
        let properties = status.responses.first().map_or(&empty, |r| &r.properties);
        let home_href = properties
            .get(&DavProperty::calendar_home_set())
            .and_then(|p| p.child(Dav::DAV, "href"))
            .map(|h| h.trimmed_text().to_string())
            .filter(|h| !h.is_empty())
            .ok_or_else(|| invalid("the server did not name a calendar home"))?;
        let mut home = self.client.resolve(&home_href, &answered_by)?;
        if !home.as_str().ends_with('/') {
            home = UrlString::parse(&format!("{}/", home.as_str())).unwrap_or(home);
        }
        let addresses: Vec<String> = properties
            .get(&DavProperty::calendar_user_address_set())
            .map(|p| p.children_named(Dav::DAV, "href"))
            .unwrap_or_default()
            .into_iter()
            .map(|h| h.trimmed_text().to_lowercase())
            .filter(|a| !a.is_empty())
            .collect();
        let options = self.client.send("OPTIONS", &home).await?;
        let dav = options.response.header("DAV").unwrap_or("");
        let auto_schedule = dav
            .split(',')
            .filter(|p| !p.is_empty())
            .any(|p| p.trim_matches([' ', '\t']).to_lowercase() == "calendar-auto-schedule");
        Ok(CalDavAccountConfig::new(
            server_url.clone(),
            username,
            principal,
            home,
            addresses,
            auto_schedule,
        ))
    }

    /// The well-known URL first; a server without it (404, 405, 501) is asked at the URL the user gave.
    async fn principal_url(&self, server_url: &UrlString) -> Outcome<UrlString> {
        let well_known = DavUrl::appending_path_component(server_url, ".well-known/caldav", false);
        for candidate in [&well_known, server_url] {
            let reply = self
                .client
                .send_with(
                    "PROPFIND",
                    candidate,
                    BTreeMap::from([("Depth".to_string(), "0".to_string())]),
                    Some(DavXml::propfind(&[DavProperty::current_user_principal()])),
                )
                .await?;
            let status = reply.response.status;
            if status == 207 {
                let multistatus = DavXml::multistatus(&reply.response.body)?;
                let href = multistatus
                    .responses
                    .first()
                    .and_then(|r| r.properties.get(&DavProperty::current_user_principal()))
                    .and_then(|p| p.child(Dav::DAV, "href"))
                    .map(|h| h.trimmed_text().to_string())
                    .filter(|h| !h.is_empty())
                    .ok_or_else(|| invalid("the server did not name a principal"))?;
                return self.client.resolve(&href, &reply.url);
            } else if candidate == &well_known && [404, 405, 501].contains(&status) {
                continue;
            } else {
                return Err(invalid(format!(
                    "the server answered {status} to discovery"
                )));
            }
        }
        Err(invalid("the server did not name a principal"))
    }
}
