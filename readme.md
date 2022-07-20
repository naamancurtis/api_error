# API Error

At the moment, this is largely a personal crate that is very much a WIP. It's not particularly fit for use and will
probably be subject to a lot of change, so use at your own peril. For now it's built with the assumption it will be used
alongside [Tracing Sprout](https://github.com/naamancurtis/tracing-sprout)

## The intent behind it

Generally when handling errors within an API there are a number of things you want to do:
1. Generate as much information as you can about the error which occurred be output to some form of logging/telemetry software
2. Handle the error internally - _if you can_
3. Generate and display some sort of sanitized error response back to your user

Previously various attempts at doing this often resulted in very large enums that were a nightmare to refactor/change
as soon as a new requirement came in. This is simply the latest attempt at simplifying this

```rust
use tracing::subscriber::set_global_default;
use tracing_sprout::TrunkLayer;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{EnvFilter, Registry};

use serde_json::{json, Value};
use thiserror::Error as ThisError;

use std::fmt;

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
    assert_eq!(json_response["category"].as_str().unwrap(), "UnexpectedServerError");
    assert_eq!(json_response["msg"].as_str().unwrap(), "An unexpected server error occurred, please try again in 5 seconds.");
}
```
