//! Stdin writer: serializes outgoing user messages as stream-json.

use crate::agents::protocol::{ControlRequest, OutgoingUserMessage};
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;
use tracing::warn;

/// Drain the user-message `rx` AND the `control_rx` channels; write each as one
/// stream-json line to `stdin`. Exits cleanly when `rx` closes (agent teardown)
/// or the writer errors. Generic so tests can use `tokio::io::duplex`.
///
/// `control_rx` carries `control_request` interrupts. We `select!` over both
/// channels (rather than draining `rx` first) so a cancel can land an interrupt
/// on stdin the instant it's sent, even while user messages sit queued — the
/// binary reads control requests out-of-band and aborts the in-flight turn
/// without dying. `control_open` disables the closed control branch so it can't
/// busy-loop on `None` once the handle (and its `control_tx`) is dropped.
///
/// `agent` is purely for diagnostics: a write error here means the
/// subprocess's stdin is gone and this agent can no longer receive ANY input
/// — user broadcasts OR peer forwards — even while its process may still be
/// alive and emitting. That asymmetric, silent death is the #4 user→HANDS
/// routing-desync failure mode (Brian goes deaf while Rain keeps receiving).
/// The old code `break`d with no log, so it was invisible. Now it's loud.
pub async fn pump_inputs<W: AsyncWrite + Unpin>(
    mut stdin: W,
    mut rx: mpsc::Receiver<OutgoingUserMessage>,
    mut control_rx: mpsc::Receiver<ControlRequest>,
    agent: String,
) {
    let mut control_open = true;
    loop {
        let line = tokio::select! {
            msg = rx.recv() => match msg {
                // The user channel closing is the agent-teardown signal.
                None => break,
                Some(m) => serde_json::to_string(&m),
            },
            ctl = control_rx.recv(), if control_open => match ctl {
                None => {
                    control_open = false;
                    continue;
                }
                Some(c) => serde_json::to_string(&c),
            },
        };
        let line = match line {
            Ok(s) => s,
            Err(err) => {
                warn!(%agent, error = %err, "serialize outgoing stdin msg");
                continue;
            }
        };
        if let Err(e) = stdin.write_all(line.as_bytes()).await {
            warn!(%agent, error = %e, "stdin write failed; input pump exiting — agent is now deaf to all input");
            break;
        }
        if let Err(e) = stdin.write_all(b"\n").await {
            warn!(%agent, error = %e, "stdin newline write failed; input pump exiting");
            break;
        }
        if let Err(e) = stdin.flush().await {
            warn!(%agent, error = %e, "stdin flush failed; input pump exiting");
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
        let (_ctl_tx, ctl_rx) = mpsc::channel(8);
        let task = tokio::spawn(pump_inputs(write, rx, ctl_rx, "test".into()));
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

    #[tokio::test]
    async fn pump_inputs_writes_control_request_out_of_band() {
        let (write, read) = tokio::io::duplex(4096);
        // No user message queued — the interrupt must still reach stdin on its own.
        let (user_tx, rx) = mpsc::channel::<OutgoingUserMessage>(8);
        let (ctl_tx, ctl_rx) = mpsc::channel(8);
        let task = tokio::spawn(pump_inputs(write, rx, ctl_rx, "test".into()));

        ctl_tx.send(ControlRequest::interrupt("r1")).await.unwrap();

        // Read the control line deterministically before tearing the channels down.
        let mut reader = BufReader::new(read);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        assert!(line.contains("\"control_request\""));
        assert!(line.contains("\"interrupt\""));
        assert!(line.contains("\"r1\""));

        drop(ctl_tx);
        drop(user_tx); // close the user channel → pump exits
        task.await.unwrap();
    }
}
