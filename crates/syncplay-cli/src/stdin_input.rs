use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
pub(super) fn spawn_local_input_receiver_legacy_compatible() -> UnboundedReceiver<String> {
    let (sender, receiver) = unbounded_channel::<String>();
    std::thread::spawn(move || {
        use std::io::BufRead;

        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            match line {
                Ok(line) => {
                    if sender.send(line).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    receiver
}

pub(super) async fn recv_local_input_line(
    local_input_rx: &mut Option<&mut UnboundedReceiver<String>>,
) -> Option<String> {
    match local_input_rx {
        Some(receiver) => receiver.recv().await,
        None => std::future::pending::<Option<String>>().await,
    }
}
