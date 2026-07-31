#[path = "../src/herdr/mod.rs"]
#[allow(dead_code)]
mod herdr;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use herdr::{HerdrClient, SocketClient};
use serde_json::json;

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct TempSocketDir(PathBuf);

impl TempSocketDir {
    fn new() -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = PathBuf::from(format!("/tmp/wb-{}-{unique}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempSocketDir {
    fn drop(&mut self) {
        let socket_path = self.0.join("herdr.sock");
        if socket_path.exists() {
            std::fs::remove_file(socket_path).unwrap();
        }
        std::fs::remove_dir(&self.0).unwrap();
    }
}

fn with_socket<T>(responses: Vec<(&'static [u8], &'static [u8])>, test: T)
where
    T: FnOnce(SocketClient),
{
    let _env_guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let tempdir = TempSocketDir::new();
    let socket_path = tempdir.0.join("herdr.sock");
    let listener = UnixListener::bind(&socket_path).unwrap();
    let (request_tx, request_rx) = mpsc::channel();

    let server = thread::spawn(move || {
        for (first, second) in responses {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut request)
                .unwrap();
            request_tx.send(request).unwrap();
            stream.write_all(first).unwrap();
            thread::sleep(Duration::from_millis(20));
            stream.write_all(second).unwrap();
        }
    });

    std::env::set_var("HERDR_SOCKET_PATH", &socket_path);
    test(SocketClient::new().unwrap());
    drop(request_rx);
    server.join().unwrap();
    std::env::remove_var("HERDR_SOCKET_PATH");
}

#[test]
fn reassembles_split_responses_and_opens_one_connection_per_call() {
    with_socket(
        vec![
            (
                br#"{"id":"1","result":{"type":"pong","version":"0."#,
                b"7.5\",\"protocol\":17,\"capabilities\":{}}}\n",
            ),
            (
                br#"{"id":"1","result":{"type":"pong","version":"0.7.6","#,
                b"\"protocol\":18,\"capabilities\":{}}}\n",
            ),
        ],
        |client| {
            let first = client.ping().unwrap();
            let second = client.ping().unwrap();
            assert_eq!(first.version, "0.7.5");
            assert_eq!(first.protocol, 17);
            assert_eq!(second.version, "0.7.6");
            assert_eq!(second.protocol, 18);
        },
    );
}

#[test]
fn turns_error_responses_into_errors_with_code_and_message() {
    with_socket(
        vec![(
            br#"{"id":"","error":{"code":"invalid_key","#,
            b"\"message\":\"unsupported key cr\"}}\n",
        )],
        |client| {
            let error = client
                .call("pane.send_input", json!({"keys": ["cr"]}))
                .unwrap_err()
                .to_string();
            assert!(error.contains("invalid_key"));
            assert!(error.contains("unsupported key cr"));
        },
    );
}

#[test]
fn parses_a_normal_response() {
    with_socket(
        vec![(br#"{"id":"1","result":{"type":"ok"}}"#, b"\n")],
        |client| {
            assert_eq!(client.call("tab.focus", json!({})).unwrap()["type"], "ok");
        },
    );
}

#[test]
fn missing_socket_environment_variable_is_an_error() {
    let _env_guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    std::env::remove_var("HERDR_SOCKET_PATH");
    let error = SocketClient::new().unwrap_err().to_string();
    assert!(error.contains("HERDR_SOCKET_PATH"));
    assert!(error.contains("Herdr"));
}
