use std::fmt;

use tracing::subscriber::set_global_default;
use tracing_sprout::TrunkLayer;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{EnvFilter, Registry};

use serde_json::{json, Value};
use thiserror::Error as ThisError;

use api_error::{e, DetailedError, ToResponse};

#[derive(Debug, ThisError)]
enum PublicError {
    #[error("An unexpected server error occurred, please try again in 5 seconds.")]
    UnexpectedServerError,
}

impl ToResponse for PublicError {
    type Response = Value;

    fn to_response(&self) -> Self::Response {
        let category = format!("{:?}", self);
        let msg = format!("{}", self);
        json!({
            "category": category,
            "msg": msg
        })
    }
}

#[derive(Debug)]
enum Category {
    IBrokeThis,
}

impl fmt::Display for Category {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

type Error = DetailedError<PublicError, Category>;

fn test() -> Result<(), Error> {
    use std::fs::File;

    let _f = File::open("random.txt").map_err(|e| {
        e!(
            e,
            PublicError::UnexpectedServerError,
            Category::IBrokeThis,
            "failed to read my amazing file"
        )
    })?;
    Ok(())
}

fn main() {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("trace"));
    let formatting_layer = TrunkLayer::new(
        "api-error-app".to_string(),
        env!("CARGO_PKG_VERSION").to_string(),
        std::io::stdout,
    );
    let subscriber = Registry::default().with(env_filter).with(formatting_layer);

    set_global_default(subscriber).expect("failed to set up global tracing subscriber");

    let e = test().unwrap_err();
    let json_response = e.to_response();
    tracing::info!(json = %serde_json::to_string_pretty(&json_response).unwrap(), "received response");
}
