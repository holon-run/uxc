use anyhow::{Context, Result};
use reqwest::Client;
use std::any::Any;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::time::Duration;

#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(test)]
static FORCE_PRIMARY_BUILD_PANIC: AtomicBool = AtomicBool::new(false);

#[cfg(test)]
pub(crate) fn set_force_primary_build_panic(value: bool) {
    FORCE_PRIMARY_BUILD_PANIC.store(value, Ordering::SeqCst);
}

fn build_client(timeout: Duration, disable_proxy: bool) -> Result<Client> {
    #[cfg(test)]
    if FORCE_PRIMARY_BUILD_PANIC.load(Ordering::SeqCst) && !disable_proxy {
        panic!("forced primary reqwest client build panic");
    }

    let mut builder = Client::builder().timeout(timeout);
    if disable_proxy {
        builder = builder.no_proxy();
    }
    builder.build().context("Failed to create HTTP client")
}

fn panic_message(payload: Box<dyn Any + Send>) -> String {
    let payload_ref = &*payload;
    if let Some(msg) = payload_ref.downcast_ref::<&str>() {
        (*msg).to_string()
    } else if let Some(msg) = payload_ref.downcast_ref::<String>() {
        msg.clone()
    } else {
        "unknown panic payload".to_string()
    }
}

pub fn build_resilient_http_client(timeout: Duration, usage: &str) -> Result<Client> {
    let primary = catch_unwind(AssertUnwindSafe(|| build_client(timeout, false)));
    match primary {
        Ok(Ok(client)) => Ok(client),
        Ok(Err(err)) => Err(err).with_context(|| {
            format!(
                "Failed to create HTTP client for {} (default proxy configuration)",
                usage
            )
        }),
        Err(payload) => {
            let message = panic_message(payload);
            tracing::warn!(
                "Reqwest client creation panicked for {}. Retrying with no_proxy(). panic={}",
                usage,
                message
            );
            build_client(timeout, true).with_context(|| {
                format!(
                    "Failed to create HTTP client for {} using no_proxy fallback",
                    usage
                )
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_resilient_http_client_succeeds_normally() {
        let client = build_resilient_http_client(Duration::from_secs(5), "unit test");
        assert!(client.is_ok());
    }

    #[test]
    fn build_resilient_http_client_falls_back_on_primary_panic() {
        set_force_primary_build_panic(true);
        let client = build_resilient_http_client(Duration::from_secs(5), "unit test fallback");
        set_force_primary_build_panic(false);
        assert!(client.is_ok());
    }
}
