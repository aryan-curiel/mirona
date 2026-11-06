//! The network gate (NGR-164). Every HTTP request any provider makes asks [`allow`] first, so
//! privacy mode is enforced where the bytes would leave, not only where providers are picked
//! ([`crate::check_privacy`]). In privacy mode only loopback hosts (Ollama, LM Studio on this
//! machine) are reachable, and cloud providers are refused even if pointed at one.
//!
//! The switch is process-wide and starts on: a process that never loaded its config can't
//! reach the network. `Engine`, the CLI and the MCP server set it from `config.toml`.
//! There is no telemetry and no update check, so nothing else in Mirona opens a connection.
//! A model downloader (NGR-174) must go through [`allow`] too.

use crate::{Error, Locality, Result};
use std::sync::atomic::{AtomicBool, Ordering};

static PRIVACY: AtomicBool = AtomicBool::new(true);

pub fn set_privacy(on: bool) {
    PRIVACY.store(on, Ordering::SeqCst);
}

pub fn privacy() -> bool {
    PRIVACY.load(Ordering::SeqCst)
}

/// Whether `url` points at this machine (`localhost`, 127.0.0.0/8, `::1`).
pub fn is_loopback(url: &str) -> bool {
    let host = host(url);
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

fn host(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    match host.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or_default(),
        None => host.split(':').next().unwrap_or_default(),
    }
}

/// Call before every request. In privacy mode, refuses cloud providers and any host that
/// isn't this machine.
pub fn allow(url: &str, locality: Locality) -> Result<()> {
    if privacy() && (locality == Locality::Cloud || !is_loopback(url)) {
        return Err(Error::PrivacyBlocked(host(url).to_string()));
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::Mutex;

    /// Tests that flip the process-wide switch take turns.
    pub static SWITCH: Mutex<()> = Mutex::new(());

    #[test]
    fn only_this_machine_is_reachable_in_privacy_mode() {
        let _g = SWITCH.lock().unwrap();
        set_privacy(true);
        assert!(allow("http://localhost:11434/api/chat", Locality::Local).is_ok());
        assert!(allow("http://[::1]:1234/v1", Locality::Local).is_ok());
        for url in [
            "https://api.openai.com/v1",
            "http://192.168.1.20:11434",
            "https://localhost.evil.com/v1",
            "https://user@127.0.0.1.example.com/",
        ] {
            assert!(
                matches!(allow(url, Locality::Local), Err(Error::PrivacyBlocked(_))),
                "{url}"
            );
        }
        assert!(allow("http://127.0.0.1:9", Locality::Cloud).is_err());
        set_privacy(false);
        assert!(allow("https://api.openai.com/v1", Locality::Cloud).is_ok());
        set_privacy(true);
    }

    /// The acceptance test: with privacy on, no provider opens a connection, even when it is
    /// pointed at a server that would answer. The same server does get a connection with
    /// privacy off, so the listener is really reachable.
    #[cfg(all(feature = "jev", feature = "ollama", feature = "openai"))]
    #[test]
    fn privacy_mode_opens_no_connection() {
        use crate::{Classifier, LlmProvider, Prompt, questions};
        let _g = SWITCH.lock().unwrap();
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        server.set_nonblocking(true).unwrap();
        let local = format!("http://{}", server.local_addr().unwrap());
        let accepted = || {
            let mut n = 0;
            while server.accept().is_ok() {
                n += 1;
            }
            n
        };
        let q = questions::segment_questions(&[]);
        let prompt = Prompt {
            system: "",
            user: "hi",
            json: false,
        };
        let jev = crate::jev::Jev::with_url("key", &local);
        // A LAN address is never tried: refused before connecting, so no timeout either.
        let lan = crate::ollama::Ollama::new("m", "http://10.255.255.1:11434/api/chat");
        let lan_llm = crate::openai::OpenAi::new("http://10.255.255.1:1234/v1", "m", None);

        set_privacy(true);
        let started = std::time::Instant::now();
        let blocked = |r: crate::Result<_>| matches!(r, Err(Error::PrivacyBlocked(_)));
        assert!(blocked(block_on(jev.ask("text", &q)).map(|_| ())));
        assert!(blocked(block_on(lan.ask("text", &q)).map(|_| ())));
        assert!(blocked(
            block_on(lan_llm.complete(&prompt, &mut |_| {})).map(|_| ())
        ));
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert_eq!(accepted(), 0, "privacy mode opened a connection");

        set_privacy(false);
        // The fake server never answers; the request only needs to reach it.
        let reached = std::thread::spawn(move || {
            let _ = block_on(jev.ask("text", &q));
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut n = 0;
        while n == 0 && std::time::Instant::now() < deadline {
            n = accepted();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        set_privacy(true);
        assert!(n > 0, "the test server was never reached");
        drop(server); // the waiting request fails and the thread ends
        reached.join().ok();
    }

    fn block_on<F: Future>(fut: F) -> F::Output {
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        let mut fut = std::pin::pin!(fut);
        loop {
            if let std::task::Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
                return v;
            }
        }
    }
}
