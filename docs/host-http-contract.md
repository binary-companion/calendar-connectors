# What a host owes the library for HTTP

The library has no HTTP client. Every connector reaches the network through the `HttpTransport` trait
(`calendar_core::http`), and the host supplies the implementation. The Swift package shipped one
(`URLSessionTransport`), and a few settings of it were pinned by tests. Those tests are not ported, because the
transport is host code; the settings are the host's obligation and are listed here. This page grows with the
connector crates: each adds the requirements of its own requests.

## Every connector

- **Answer with any HTTP response.** A 4xx or 5xx answer is a normal `HttpResponse`, not an error.
- **Failures.** A transport failure (no route, TLS, timeout) is `Failure::Source(SourceError::Network(message))`.
  A request that was cancelled is `Failure::Cancelled`, never a network error.
- **No cache.** Sync tokens make every poll a new URL, so a cache would only pile up responses and keep
  authenticated bodies on disk. The request must ignore any local cache, and the session must have none.
- **Header names.** `HttpResponse::new` lowercases them; the host may pass them as received.
- **Timeouts.** The Swift transport set none of its own and ran on the defaults of an ephemeral `URLSession`
  (60 seconds for a request, seven days for the whole resource). A host should set finite timeouts.

## iCal subscription (`calendar-ical-subscription`)

The feed link is a credential (ADR 0018): whoever holds it can read the calendar. The Swift code built its transport with
`URLSessionTransport(configuration: feedSessionConfiguration, followsRedirects: false)`, and these are the settings:

- **The transport must not follow redirects.** It returns a 3xx answer as the response, with its `Location` header.
  `FeedFetcher` follows redirects itself, hop by hop, and refuses a target that is not `https`, a target with a user
  name or password, and more than five redirects. It sends the validators (`If-None-Match`, `If-Modified-Since`) to the
  original link only. A transport that follows redirects on its own would skip those checks and send a request,
  whose address is the credential, to places the library never saw.
- **No URL cache.** The cache is keyed by the full link and keeps the body (Swift: `urlCache = nil` and
  `requestCachePolicy = .reloadIgnoringLocalCacheData`).
- **No cookies.** None are sent and none are stored (`httpShouldSetCookies = false`, `httpCookieStorage = nil`).
- **Nothing on disk.** The session is ephemeral (`URLSessionConfiguration.ephemeral`): nothing about a request may reach
  disk.
- **Keep the link out of failures.** The library replaces a `Network` message that repeats the link, its path or its
  query, but a host should not put the link in one at all, nor in its own logs.

`ICalSubscriptionKind::new` repeats the first requirement in its documentation.
