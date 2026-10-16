//! The agent control channel: newline-delimited JSON over TCP on 127.0.0.1, authenticated
//! with a token. Requests are forwarded to the UI thread, which answers them (see
//! `docs/control-protocol.md`).

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use serde_json::{Value, json};

/// One request waiting for the UI thread.
pub struct Pending {
    /// Method name.
    pub method: String,
    /// Parameters.
    pub params: Value,
    /// Where to send the result (`Ok(result)` or `Err(message)`).
    pub reply: Sender<Result<Value, String>>,
}

/// The running server.
pub struct Server {
    rx: Receiver<Pending>,
    /// The port it listens on.
    pub port: u16,
}

impl Server {
    /// Next request for the UI thread.
    pub fn try_recv(&self) -> Option<Pending> {
        self.rx.try_recv().ok()
    }
}

/// A fresh random-ish 256-bit token (hex), from the OS's hash randomness and the clock.
pub fn new_token() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let mut out = String::new();
    for i in 0..4u64 {
        let mut h = RandomState::new().build_hasher();
        h.write_u64(i);
        h.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos()),
        );
        h.write_u32(std::process::id());
        out.push_str(&format!("{:016x}", h.finish()));
    }
    out
}

/// Reads the token from `file`, creating it (mode 0600 on Unix) when missing.
pub fn token_from_file(file: &Path) -> std::io::Result<String> {
    if let Ok(t) = std::fs::read_to_string(file) {
        let t = t.trim().to_string();
        if !t.is_empty() {
            return Ok(t);
        }
    }
    let token = new_token();
    std::fs::write(file, &token)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(token)
}

/// Starts listening on 127.0.0.1:`port`. `wake` is called when a request arrives.
pub fn start(
    port: u16,
    token: String,
    wake: impl Fn() + Send + Sync + Clone + 'static,
) -> std::io::Result<Server> {
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    let port = listener.local_addr()?.port();
    let (tx, rx) = channel::<Pending>();
    std::thread::Builder::new()
        .name("texcraft-control".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let tx = tx.clone();
                let token = token.clone();
                let wake = wake.clone();
                let _ = std::thread::Builder::new()
                    .name("texcraft-control-conn".into())
                    .spawn(move || {
                        let _ = serve(stream, &token, &tx, &wake);
                    });
            }
        })?;
    Ok(Server { rx, port })
}

fn serve(
    stream: TcpStream,
    token: &str,
    tx: &Sender<Pending>,
    wake: &dyn Fn(),
) -> std::io::Result<()> {
    let mut writer = stream.try_clone()?;
    let reader = BufReader::new(stream);
    let mut authed = false;
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let req: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                writeln!(
                    writer,
                    "{}",
                    json!({"id": null, "ok": false, "error": format!("bad JSON: {e}")})
                )?;
                continue;
            }
        };
        let id = req.get("id").cloned().unwrap_or(Value::Null);
        let method = req
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let params = req.get("params").cloned().unwrap_or(Value::Null);
        if !authed {
            let ok = method == "auth" && params.get("token").and_then(Value::as_str) == Some(token);
            if ok {
                authed = true;
                writeln!(
                    writer,
                    "{}",
                    json!({"id": id, "ok": true, "result": "authenticated"})
                )?;
                continue;
            }
            writeln!(
                writer,
                "{}",
                json!({"id": id, "ok": false, "error": "authenticate first: {\"method\":\"auth\",\"params\":{\"token\":…}}"})
            )?;
            return Ok(());
        }
        if method == "auth" {
            writeln!(
                writer,
                "{}",
                json!({"id": id, "ok": false, "error": "already authenticated"})
            )?;
            continue;
        }
        let (rtx, rrx) = channel();
        if tx
            .send(Pending {
                method,
                params,
                reply: rtx,
            })
            .is_err()
        {
            return Ok(());
        }
        wake();
        let reply = match rrx.recv_timeout(Duration::from_secs(120)) {
            Ok(Ok(result)) => json!({"id": id, "ok": true, "result": result}),
            Ok(Err(error)) => json!({"id": id, "ok": false, "error": error}),
            Err(_) => json!({"id": id, "ok": false, "error": "timeout: no reply in 120s"}),
        };
        writeln!(writer, "{reply}")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_64_hex_chars_and_differ() {
        let a = new_token();
        let b = new_token();
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn requires_auth_then_forwards() {
        let server = start(0, "secret".into(), || {}).unwrap();
        let port = server.port;
        let client = std::thread::spawn(move || {
            let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
            let mut r = BufReader::new(s.try_clone().unwrap());
            let mut line = String::new();
            writeln!(
                s,
                r#"{{"id":1,"method":"auth","params":{{"token":"secret"}}}}"#
            )
            .unwrap();
            r.read_line(&mut line).unwrap();
            assert!(line.contains("\"ok\":true"), "{line}");
            line.clear();
            writeln!(s, r#"{{"id":2,"method":"ping","params":{{}}}}"#).unwrap();
            r.read_line(&mut line).unwrap();
            line
        });
        // Answer the forwarded request on "the UI thread".
        let pending = loop {
            if let Some(p) = server.try_recv() {
                break p;
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(pending.method, "ping");
        pending.reply.send(Ok(json!("pong"))).unwrap();
        let line = client.join().unwrap();
        assert!(line.contains("pong") && line.contains("\"id\":2"), "{line}");
    }

    #[test]
    fn rejects_bad_token() {
        let server = start(0, "secret".into(), || {}).unwrap();
        let mut s = TcpStream::connect(("127.0.0.1", server.port)).unwrap();
        let mut r = BufReader::new(s.try_clone().unwrap());
        writeln!(s, r#"{{"id":1,"method":"command.run","params":{{}}}}"#).unwrap();
        let mut line = String::new();
        r.read_line(&mut line).unwrap();
        assert!(line.contains("authenticate first"));
        assert!(server.try_recv().is_none());
    }
}
