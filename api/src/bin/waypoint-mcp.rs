//! `waypoint-mcp --socket <path>`: the stdio MCP server `claude -p` starts from an agent
//! call's generated config. It only relays: stdin goes to the API's per-call Unix socket
//! and the socket's replies go to stdout. It reads no env, opens no database and holds no
//! Waypoint logic; the protocol is served by `waypoint_api::mcp` inside the API.

use std::path::PathBuf;
use std::process::ExitCode;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let socket = match (args.next(), args.next(), args.next()) {
        (Some(flag), Some(path), None) if flag == "--socket" => PathBuf::from(path),
        _ => {
            eprintln!("usage: waypoint-mcp --socket <path>");
            return ExitCode::FAILURE;
        }
    };
    let stream = match UnixStream::connect(&socket).await {
        Ok(stream) => stream,
        Err(err) => {
            eprintln!(
                "waypoint-mcp: cannot connect to {}: {err}",
                socket.display()
            );
            return ExitCode::FAILURE;
        }
    };
    let (mut from_socket, mut to_socket) = stream.into_split();

    // stdin to the socket; at EOF, close the write half so the server sees the end.
    tokio::spawn(async move {
        let mut stdin = tokio::io::stdin();
        let _ = tokio::io::copy(&mut stdin, &mut to_socket).await;
        let _ = to_socket.shutdown().await;
    });

    // The socket to stdout, flushing each chunk, until the server closes.
    let mut stdout = tokio::io::stdout();
    let mut buf = vec![0u8; 64 * 1024];
    let code = loop {
        match from_socket.read(&mut buf).await {
            Ok(0) => break 0,
            Ok(n) => {
                if stdout.write_all(&buf[..n]).await.is_err() || stdout.flush().await.is_err() {
                    break 1;
                }
            }
            Err(err) => {
                eprintln!("waypoint-mcp: cannot read from the socket: {err}");
                break 1;
            }
        }
    };
    // A blocking stdin read can't be cancelled, so exit instead of waiting on it.
    std::process::exit(code);
}
