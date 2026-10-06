/// A stream event that could not be understood. The message names the problem
/// and never repeats the payload, so a key-like string in a server reply cannot
/// reach a log through this type.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("stream event is not valid JSON")]
    InvalidJson,
    #[error("stream event is missing `{0}`")]
    Missing(&'static str),
}
