//! Schemes the app may open from calendar-supplied meeting data. Port of `JoinURLPolicy.swift`.

use crate::url_string::UrlString;

/// Decides which links from calendar data the app may open as a meeting join link.
pub enum JoinUrlPolicy {}

impl JoinUrlPolicy {
    /// True for `http` and `https` links, and for `zoommtg` links to a `zoom.us` or `zoom.com` host. False when the
    /// scheme or host is missing, or the URL carries a user name or password.
    pub fn is_allowed(url: &UrlString) -> bool {
        let (Some(scheme), Some(host)) = (url.scheme(), url.host()) else {
            return false;
        };
        let (scheme, host) = (scheme.to_lowercase(), host.to_lowercase());
        if host.is_empty() || url.user().is_some() || url.password().is_some() {
            return false;
        }
        match scheme.as_str() {
            "http" | "https" => true,
            "zoommtg" => {
                host == "zoom.us"
                    || host == "zoom.com"
                    || host.ends_with(".zoom.us")
                    || host.ends_with(".zoom.com")
            }
            _ => false,
        }
    }
}
