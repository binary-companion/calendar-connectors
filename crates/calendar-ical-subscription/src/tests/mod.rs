#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod fixtures;

mod diagnostics_tests;
mod feed_fetcher_tests;
mod feed_location_probe_tests;
mod feed_location_tests;
mod feed_parser_tests;
mod garbage_input_tests;
mod kind_tests;
mod leak_tests;
mod live_tests;
mod notice_tests;
mod overlap_tests;
mod persisted_format_tests;
mod retention_tests;
mod rust_additions_tests;
mod source_tests;
