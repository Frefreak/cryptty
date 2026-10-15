use std::process::{Command, Stdio};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::time::{Duration, Instant};

use winit::window::{UserAttentionType, Window};

#[derive(Default)]
pub(super) struct Notifications {
    sender: Option<SyncSender<String>>,
    last_attention: Option<Instant>,
    last_message: Option<Instant>,
}

impl Notifications {
    pub(super) fn notify(&mut self, window: &Window, message: Option<String>) {
        if window.has_focus() {
            return;
        }
        if allow_now(&mut self.last_attention) {
            window
                .request_user_attention(Some(UserAttentionType::Informational));
        }
        let Some(message) = message else { return };
        if !allow_now(&mut self.last_message) {
            return;
        }
        let sender = self.sender.get_or_insert_with(|| {
            let (sender, receiver) = sync_channel::<String>(4);
            std::thread::spawn(move || {
                for message in receiver {
                    let mut command = notification_command(&message);
                    match command
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status()
                    {
                        Ok(status) if status.success() => (),
                        Ok(status) => eprintln!(
                            "Cryptty: notification exited with {status}"
                        ),
                        Err(error) => {
                            eprintln!("Cryptty: notification failed: {error}")
                        }
                    }
                }
            });
            sender
        });
        let _ = sender.try_send(message);
    }
}

fn allow_now(last: &mut Option<Instant>) -> bool {
    if last.is_some_and(|last| last.elapsed() < Duration::from_secs(1)) {
        return false;
    }
    *last = Some(Instant::now());
    true
}

#[cfg(target_os = "macos")]
fn notification_command(message: &str) -> Command {
    let mut command = Command::new("/usr/bin/osascript");
    command.args([
        "-e",
        "on run argv\ndisplay notification (item 1 of argv) with title \"Cryptty\"\nend run",
        "--",
        message,
    ]);
    command
}

#[cfg(not(target_os = "macos"))]
fn notification_command(message: &str) -> Command {
    let mut command = Command::new("notify-send");
    command.args(["--app-name=Cryptty", "--", "Cryptty", message]);
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bell_attention_does_not_suppress_notification_text() {
        let mut notifications = Notifications::default();
        assert!(allow_now(&mut notifications.last_attention));
        assert!(allow_now(&mut notifications.last_message));
        assert!(!allow_now(&mut notifications.last_message));
        assert!(!allow_now(&mut notifications.last_attention));
    }

    #[test]
    #[ignore = "sends a real desktop notification"]
    fn desktop_notification_command_succeeds() {
        assert!(
            notification_command("Cryptty verification completed")
                .status()
                .unwrap()
                .success()
        );
    }

    #[test]
    fn notification_text_is_passed_as_an_argument() {
        let message = "quote \"; $(touch should-not-exist)\nnext line";
        let command = notification_command(message);
        assert_eq!(command.get_args().last().unwrap(), message);
        assert!(
            !command
                .get_args()
                .take(command.get_args().len() - 1)
                .any(|arg| arg.to_string_lossy().contains(message))
        );
    }
}
