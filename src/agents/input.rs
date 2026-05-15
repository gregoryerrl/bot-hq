//! Stdin writer: serializes outgoing user messages as stream-json.

use crate::agents::protocol::OutgoingUserMessage;
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;
use tracing::warn;

/// Drain the `rx` channel; write each message as one stream-json line to
/// `stdin`. Exits cleanly when `rx` closes or the writer errors.
/// Generic so tests can use `tokio::io::duplex`.
pub async fn pump_inputs<W: AsyncWrite + Unpin>(
    mut stdin: W,
    mut rx: mpsc::Receiver<OutgoingUserMessage>,
) {
    while let Some(msg) = rx.recv().await {
        let line = match serde_json::to_string(&msg) {
            Ok(s) => s,
            Err(err) => {
                warn!(error = %err, "serialize outgoing user msg");
                continue;
            }
        };
        if stdin.write_all(line.as_bytes()).await.is_err() {
            break;
        }
        if stdin.write_all(b"\n").await.is_err() {
            break;
        }
        if stdin.flush().await.is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, BufReader};

    #[tokio::test]
    async fn pump_inputs_writes_line_per_message() {
        let (write, read) = tokio::io::duplex(4096);
        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(pump_inputs(write, rx));
        tx.send(OutgoingUserMessage::text("hello")).await.unwrap();
        tx.send(OutgoingUserMessage::text("world")).await.unwrap();
        drop(tx);
        task.await.unwrap();

        let mut reader = BufReader::new(read);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        assert!(line.contains("\"hello\""));
        let mut line2 = String::new();
        reader.read_line(&mut line2).await.unwrap();
        assert!(line2.contains("\"world\""));
    }
}
