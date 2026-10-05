//! Single-instance endpoint for the `softwarefix://` callback.
//!
//! When LMN Flash owns the scheme (see [`crate::protocol`]), the login flow can
//! use the system browser: the browser redirects to `softwarefix://callback…`
//! and the operating system starts another copy of this executable with that
//! URL as an argument. That copy hands the URL to the already-running instance
//! over a loopback socket and exits, so the login completes in the window the
//! user is looking at instead of in a second one.
//!
//! The endpoint is advertised in `<config>/instance` (an ephemeral loopback
//! port plus a per-process token). A file left behind by a crashed process is
//! detected by trying to connect to it, so nothing has to be cleaned up on
//! exit.

use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::time::Duration;

use iced::futures::channel::mpsc::{UnboundedReceiver, UnboundedSender};

/// The first word of both the endpoint file and the request header.
const MAGIC: &str = "lmnflash-instance";

/// Bumped when the wire format changes; a mismatch is ignored.
const VERSION: &str = "1";

/// Longest callback URL accepted, so a malformed request cannot grow the
/// process without bound.
const MAX_URL: usize = 16 * 1024;

/// How long a connect, read or write may take before forwarding is given up.
const TIMEOUT: Duration = Duration::from_secs(2);

/// What [`start`] decided for this process.
pub enum Endpoint {
    /// This process owns the endpoint; forwarded URLs arrive on the receiver.
    Primary(UnboundedReceiver<String>),
    /// Another instance owns the endpoint.
    Secondary,
    /// No endpoint could be created; the app runs without callback forwarding.
    Disabled,
}

/// Starts the single-instance endpoint, handing `url` to a running instance
/// when there is one.
pub fn start(url: Option<&str>) -> Endpoint {
    if let Some((port, token)) = live_endpoint() {
        match url {
            Some(url) if forward(port, &token, url) => return Endpoint::Secondary,
            // A plain second launch: let it open its own window, but leave the
            // endpoint (and therefore the callbacks) with the first instance.
            None => return Endpoint::Secondary,
            // A running instance that did not accept the URL is not usable;
            // fall through and take the endpoint over.
            Some(_) => {}
        }
    }

    match serve() {
        Ok((port, token, receiver)) => {
            if write_endpoint(&render_endpoint(port, &token)).is_ok() {
                Endpoint::Primary(receiver)
            } else {
                Endpoint::Disabled
            }
        }
        Err(error) => {
            eprintln!("[instance] no single-instance endpoint: {error}");
            Endpoint::Disabled
        }
    }
}

/// Binds the loopback listener and spawns the accept thread.
fn serve() -> std::io::Result<(u16, String, UnboundedReceiver<String>)> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let port = listener.local_addr()?.port();
    let token = uuid::Uuid::new_v4().simple().to_string();
    let (sender, receiver) = iced::futures::channel::mpsc::unbounded();

    let accept_token = token.clone();

    std::thread::Builder::new()
        .name("lmnflash-instance".to_string())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let _ = handle_connection(stream, &accept_token, &sender);
            }
        })?;

    Ok((port, token, receiver))
}

/// Reads one request and, when it is ours, pushes the callback URL into the UI
/// channel and acknowledges it.
fn handle_connection(
    mut stream: TcpStream,
    token: &str,
    sender: &UnboundedSender<String>,
) -> std::io::Result<()> {
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;

    let mut header = String::new();
    let mut url = String::new();

    {
        let mut reader = BufReader::new(&mut stream);

        if reader.read_line(&mut header)? == 0 || reader.read_line(&mut url)? == 0 {
            return Ok(());
        }
    }

    let expected = format!("{MAGIC} {VERSION} {token}");

    if header.trim() != expected || url.len() > MAX_URL {
        return Ok(());
    }

    let _ = sender.unbounded_send(url.trim().to_string());
    stream.write_all(b"ok\n")?;

    Ok(())
}

/// Hands `url` to the instance listening on `port`, reporting whether it was
/// accepted.
fn forward(port: u16, token: &str, url: &str) -> bool {
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));

    let Ok(mut stream) = TcpStream::connect_timeout(&address, TIMEOUT) else {
        return false;
    };

    let _ = stream.set_read_timeout(Some(TIMEOUT));
    let _ = stream.set_write_timeout(Some(TIMEOUT));

    let request = format!("{MAGIC} {VERSION} {token}\n{url}\n");

    if stream.write_all(request.as_bytes()).is_err() || stream.flush().is_err() {
        return false;
    }

    let mut reply = String::new();

    matches!(
        BufReader::new(&stream).read_line(&mut reply),
        Ok(read) if read > 0 && reply.trim() == "ok"
    )
}

/// The file that advertises the primary instance's endpoint.
fn endpoint_path() -> PathBuf {
    crate::config::config_dir().join("instance")
}

/// Renders the endpoint file contents.
fn render_endpoint(port: u16, token: &str) -> String {
    format!("{MAGIC} {VERSION} {port} {token}\n")
}

/// Parses the endpoint file contents (see [`render_endpoint`]).
fn parse_endpoint(raw: &str) -> Option<(u16, String)> {
    let mut parts = raw.split_whitespace();

    if parts.next()? != MAGIC || parts.next()? != VERSION {
        return None;
    }

    let port = parts.next()?.parse().ok()?;
    let token = parts.next()?.to_string();

    Some((port, token))
}

/// Reads the endpoint file and confirms that something is actually listening.
fn live_endpoint() -> Option<(u16, String)> {
    let (port, token) = parse_endpoint(&std::fs::read_to_string(endpoint_path()).ok()?)?;

    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    TcpStream::connect_timeout(&address, TIMEOUT).ok()?;

    Some((port, token))
}

/// Writes the endpoint file atomically.
fn write_endpoint(contents: &str) -> std::io::Result<()> {
    let path = endpoint_path();

    if let Some(directory) = path.parent() {
        std::fs::create_dir_all(directory)?;
    }

    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, &path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_endpoint_file_round_trips() {
        let raw = render_endpoint(51234, "abc123");

        assert_eq!(parse_endpoint(&raw), Some((51234, "abc123".to_string())));
    }

    #[test]
    fn foreign_endpoint_files_are_rejected() {
        assert_eq!(parse_endpoint(""), None);
        assert_eq!(parse_endpoint("other-tool 1 51234 abc"), None);
        assert_eq!(parse_endpoint("lmnflash-instance 2 51234 abc"), None);
        assert_eq!(parse_endpoint("lmnflash-instance 1 not-a-port abc"), None);
    }

    #[test]
    fn a_callback_reaches_the_listening_instance() {
        let (port, token, mut receiver) = serve().expect("could not bind a loopback port");

        assert!(forward(
            port,
            &token,
            "softwarefix://callback?Authorization=abc"
        ));

        assert_eq!(
            receive(&mut receiver).as_deref(),
            Some("softwarefix://callback?Authorization=abc")
        );
    }

    #[test]
    fn a_wrong_token_is_not_delivered() {
        let (port, _token, mut receiver) = serve().expect("could not bind a loopback port");

        assert!(!forward(port, "wrong", "softwarefix://callback?Authorization=abc"));
        assert!(receiver.try_recv().is_err());
    }

    /// Polls the receiver for up to a second: the accept thread sends from
    /// another thread, so a single `try_recv` can race.
    fn receive(receiver: &mut UnboundedReceiver<String>) -> Option<String> {
        for _ in 0..100 {
            match receiver.try_recv() {
                Ok(url) => return Some(url),
                // Nothing yet: the accept thread may not have sent.
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }

        None
    }
}
