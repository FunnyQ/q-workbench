use serde_json::json;

use crate::herdr::HerdrClient;

/// Attempts to show an error without replacing the error being reported.
pub fn notify(client: &dyn HerdrClient, title: &str, body: &str) {
    let _ = client.notification_show(json!({
        "title": title,
        "body": body,
        "position": "bottom-right",
        "sound": "none",
    }));
}

pub fn notify_and_exit(client: &dyn HerdrClient, title: &str, body: &str) -> ! {
    notify(client, title, body);
    std::process::exit(1);
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::notify;
    use crate::herdr::FakeClient;

    #[test]
    fn notify_sends_the_expected_notification() {
        let client = FakeClient::default();

        notify(&client, "Launch failed", "Could not start agent");

        assert_eq!(
            client.calls.into_inner(),
            vec![(
                "notification.show".to_owned(),
                json!({
                    "title": "Launch failed",
                    "body": "Could not start agent",
                    "position": "bottom-right",
                    "sound": "none",
                }),
            )]
        );
    }

    #[test]
    fn notify_swallows_client_failures() {
        let client = FakeClient::default();
        client.queue_error("notification.show", "unavailable", "notifications disabled");

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            notify(&client, "Launch failed", "Could not start agent");
        }));

        assert!(result.is_ok());
        assert_eq!(client.calls.borrow().len(), 1);
    }
}
