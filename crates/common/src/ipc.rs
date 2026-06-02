// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! IPC plumbing helpers shared across crates.
//!
//! Brume's threads communicate over bounded crossbeam channels. The
//! audio worker, MIDI input thread, and iced UI thread all `try_send`
//! on these channels and treat send failures as drops — the channel
//! is bounded, so a saturated channel must not block any of them.
//!
//! The default `let _ = tx.try_send(msg)` pattern silently swallows
//! drops, which is fine in normal operation but offers no diagnostic
//! when load actually exceeds capacity. `try_send_or_log!` is the
//! drop-in replacement: same non-blocking semantics, but the first
//! drop and every Nth drop afterward log the file:line and message
//! variant so a tester reporting "the UI started lagging" has
//! something concrete to send back.

/// Non-blocking try-send with rate-limited per-site logging on
/// drops. Use it anywhere the old pattern `let _ = tx.try_send(msg)`
/// would have appeared.
///
/// Each macro expansion has its own static drop counter, so each
/// call site is rate-limited independently. Log cadence: every 64th
/// drop (sites 1, 65, 129, …). Tuned to surface real saturation
/// events without flooding stderr if a channel is genuinely
/// hammered.
#[macro_export]
macro_rules! try_send_or_log {
    ($tx:expr, $msg:expr $(,)?) => {{
        match ($tx).try_send($msg) {
            Ok(()) => {}
            Err(e) => {
                static DROP_COUNT: ::std::sync::atomic::AtomicU64 =
                    ::std::sync::atomic::AtomicU64::new(0);
                let n = DROP_COUNT.fetch_add(1, ::std::sync::atomic::Ordering::Relaxed);
                if n % 64 == 0 {
                    ::std::eprintln!(
                        "{}:{} channel send dropped (count={} at this site): {:?}",
                        ::std::file!(),
                        ::std::line!(),
                        n + 1,
                        e,
                    );
                }
            }
        }
    }};
}

#[cfg(test)]
mod tests {
    use crossbeam_channel::bounded;

    #[test]
    fn try_send_or_log_succeeds_when_channel_has_room() {
        let (tx, rx) = bounded::<u32>(2);
        crate::try_send_or_log!(tx, 1);
        crate::try_send_or_log!(tx, 2);
        assert_eq!(rx.try_recv().unwrap(), 1);
        assert_eq!(rx.try_recv().unwrap(), 2);
    }

    #[test]
    fn try_send_or_log_drops_when_channel_full() {
        let (tx, _rx) = bounded::<u32>(1);
        crate::try_send_or_log!(tx, 1);
        // Capacity exceeded — this drops, no panic, no block.
        crate::try_send_or_log!(tx, 2);
    }
}
