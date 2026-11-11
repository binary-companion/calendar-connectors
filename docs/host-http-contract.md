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

## CalDAV (`calendar-caldav`)

`ICloudConnectorKind::new` and `CalDavConnectorKind::new` take the host's `HttpTransport`. The Swift code built its
default with `URLSessionTransport(followsRedirects: false)`, and these are the requirements:

- **The transport must not follow redirects.** It returns a 3xx answer as the response, with its `Location` header.
  `WebDavClient` follows redirects itself, hop by hop (301, 302, 303, 307, 308; at most five), and checks every hop
  before any credential is sent: the target must be `https` (plain `http` only to `localhost` or `127.0.0.1`) and its
  host must be the account's host base or a subdomain of it (`icloud.com` for iCloud, the entered server's host for
  another server). A 303 becomes a `GET` without a body. A transport that follows redirects on its own would send the
  HTTP Basic `Authorization` header (the account password) to places the library never checked.
- **Do not add, change or log the `Authorization` header.** The library sends `Basic` over the standard base64
  alphabet with padding. The host must keep the header (and the request URL's credentials, if any) out of its own logs
  and out of the `Network` messages it returns; the library never puts the password into an error or a `Debug` output.
- **Pass methods and headers through unchanged.** The requests use `PROPFIND`, `REPORT`, `OPTIONS`, `GET`, `PUT` and
  `DELETE`, and the headers `Depth`, `Content-Type`, `If-Match` and `If-None-Match`. `412` and `207` answers are
  ordinary responses.
- **Return the answer's `ETag`, `DAV`, `Location` and `Retry-After` headers** (names are lowercased by
  `HttpResponse::new`).
- **No cache, no cookies, nothing on disk** (the general requirements above): every request carries credentials and
  sync tokens, and a cached body would keep a calendar on disk.

## Google (`calendar-google`)

`GoogleConnectorKind::new` takes the host's `HttpTransport`. The Swift code built its default with
`URLSessionTransport()`, and these are the requirements:

- **Redirects are not needed.** The Google endpoints (`accounts.google.com`, `oauth2.googleapis.com`,
  `www.googleapis.com`) answer directly, and the library treats any 3xx as an unexpected status. A transport that
  follows redirects does no harm here, because the only secret the library sends, the `Authorization: Bearer` access
  token, goes to Google's own hosts.
- **Do not add, change or log the `Authorization` header.** The library sends `Bearer <access token>` and, at sign-in,
  the OAuth client id and secret and the authorization code in the body of a `POST` to the token endpoint
  (`application/x-www-form-urlencoded`). The host must keep these out of its own logs and out of the `Network`
  messages it returns; the library never puts a token, the client secret or the refresh token into an error, a
  `Debug` output or a diagnostic.
- **Pass methods and headers through unchanged.** The requests use `GET`, `POST`, `PATCH` and `DELETE`, and the headers
  `Accept`, `Content-Type`, `If-Match` and `Authorization`. Status codes 400, 401, 403, 404, 409, 410, 412 and 429 are
  ordinary responses that the library interprets (a 410 on a sync token means "start over", a 412 on `If-Match` means
  "the event changed").
- **Return the answer's `Retry-After` header** (the name is lowercased by `HttpResponse::new`): the library honors it
  exactly when it backs off from a rate limit, and waits `0.5` to `1` second times a power of two otherwise.
- **No cache, no cookies, nothing on disk** (the general requirements above): every request carries a bearer token and
  sync tokens, and a cached body would keep a calendar on disk.

## Microsoft (`calendar-microsoft`)

`MicrosoftConnectorKind::new` and `MicrosoftBrokeredConnectorKind::new` take the host's `HttpTransport`. The Swift code
built its default with `URLSessionTransport()`, and these are the requirements. The browser kind talks to
`login.microsoftonline.com` (sign-in and token refresh) and `graph.microsoft.com`; the brokered kind talks to
`graph.microsoft.com` only.

- **Redirects are not needed.** Both hosts answer directly, and the library treats any 3xx as an unexpected status. A
  transport that follows redirects must never send the `Authorization` header to a host other than the one it was
  built for. The library itself follows an `@odata.nextLink` or delta link only when it is `https` on
  `graph.microsoft.com`, and otherwise stops with `SourceError::InvalidResponse("unexpected link host")`, so the bearer
  token never leaves Graph's host through a link.
- **Do not add, change or log the `Authorization` header.** The library sends `Bearer <access token>` to Graph. At
  sign-in the browser kind also sends the OAuth client id, the authorization code and the PKCE verifier (or, to refresh,
  the refresh token) in the body of a `POST` to the token endpoint (`application/x-www-form-urlencoded`); it is a
  public client, so there is no client secret. The host must keep all of these out of its own logs and out of the
  `Network` messages it returns; the library never puts a token, a code or a verifier into an error, a `Debug` output
  or a diagnostic. The brokered kind never sees a refresh token: the access tokens come from the host's
  `ExternalTokenSource`, go only into the `Bearer` header, and nothing is stored.
- **Pass methods, headers and addresses through unchanged.** The requests use `GET`, `POST`, `PATCH` and `DELETE`, and
  the headers `Accept`, `Content-Type`, `Authorization` and `Prefer`. `Prefer` carries several preferences in one
  header, quotes included, for example `IdType="ImmutableId", outlook.timezone="Pacific Standard Time",
  outlook.body-content-type="text"` or `odata.maxpagesize=200`; the host must send it as given (not merge it with its
  own, not drop the quotes) because Graph returns times in the zone named there. The request address is sent as the
  library wrote it: `$select`, `$top`, `$filter`, `$deltatoken` and `$skiptoken` keep their `$`, and the commas and
  colons of a query value stay unescaped, exactly as `URLComponents` would write them. Delta and next links are opaque
  and must not be re-encoded or normalized.
- **Status codes are ordinary answers.** 400, 401, 403, 404, 409, 410, 412, 429, 503 and 504 are interpreted by the
  library (a 410, or a 400 whose Graph error code names an invalid delta token, means "start the delta over"; a 401 is
  retried once with a fresh token). `202` (an RSVP) and `204` (a delete) answer with an empty body and are successes.
- **Return the answer's `Retry-After` header** (the name is lowercased by `HttpResponse::new`): on a 429, 503 or 504
  the library honors it exactly when it backs off, and otherwise waits `0.5` to `1` second times a power of two, at most
  three times.
- **No cache, no cookies, nothing on disk** (the general requirements above): every request carries a bearer token,
  and a cached body would keep a calendar on disk.
