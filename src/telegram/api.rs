//! The part of TDLib's API that FinchGram uses, as serde types, written against td_api.tl of
//! [`super::TDLIB_VERSION`]. Every TDLib object names its type in "@type".
//!
//! Fields that are not listed here are ignored, so a newer TDLib may add fields freely. One that
//! renames or removes a field we read fails to deserialize: the update is logged and dropped
//! (process.rs). When TDLib is upgraded, compare its td_api.tl with the types here.
//!
//! Requests are written with serde_json's json! macro where they are sent: they are TDLib's own
//! objects, and td_api.tl is their documentation.

use serde::Deserialize;

/// Something changed. Only the updates FinchGram follows are listed; every other kind is `Other`
/// and is dropped as soon as it is read.
#[derive(Debug, Deserialize)]
#[serde(tag = "@type")]
pub enum Update {
    #[serde(rename = "updateAuthorizationState")]
    AuthorizationState { authorization_state: AuthorizationState },
    #[serde(rename = "updateConnectionState")]
    ConnectionState { state: ConnectionState },
    #[serde(other)]
    Other,
}

/// Where logging in is.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "@type")]
pub enum AuthorizationState {
    #[serde(rename = "authorizationStateWaitTdlibParameters")]
    WaitTdlibParameters,
    #[serde(rename = "authorizationStateWaitPhoneNumber")]
    WaitPhoneNumber,
    #[serde(rename = "authorizationStateWaitPremiumPurchase")]
    WaitPremiumPurchase,
    #[serde(rename = "authorizationStateWaitEmailAddress")]
    WaitEmailAddress,
    #[serde(rename = "authorizationStateWaitEmailCode")]
    WaitEmailCode,
    #[serde(rename = "authorizationStateWaitCode")]
    WaitCode,
    /// Logging in with a QR code: `link` is what the code shows, confirmed on a device that is
    /// already logged in.
    #[serde(rename = "authorizationStateWaitOtherDeviceConfirmation")]
    WaitOtherDeviceConfirmation { link: String },
    #[serde(rename = "authorizationStateWaitRegistration")]
    WaitRegistration,
    #[serde(rename = "authorizationStateWaitPassword")]
    WaitPassword { password_hint: String },
    #[serde(rename = "authorizationStateReady")]
    Ready,
    #[serde(rename = "authorizationStateLoggingOut")]
    LoggingOut,
    #[serde(rename = "authorizationStateClosing")]
    Closing,
    /// TDLib has closed. finchgram-tdlib ends right after saying so.
    #[serde(rename = "authorizationStateClosed")]
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(tag = "@type")]
pub enum ConnectionState {
    #[serde(rename = "connectionStateWaitingForNetwork")]
    WaitingForNetwork,
    #[serde(rename = "connectionStateConnectingToProxy")]
    ConnectingToProxy,
    #[serde(rename = "connectionStateConnecting")]
    Connecting,
    #[serde(rename = "connectionStateUpdating")]
    Updating,
    #[serde(rename = "connectionStateReady")]
    Ready,
}

/// TDLib's answer when a request failed.
#[derive(Debug, Clone, Deserialize)]
pub struct TdError {
    pub code: i32,
    pub message: String,
}

/// The value of an option (getOption, updateOption).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "@type")]
pub enum OptionValue {
    #[serde(rename = "optionValueBoolean")]
    Boolean { value: bool },
    #[serde(rename = "optionValueEmpty")]
    Empty,
    #[serde(rename = "optionValueInteger")]
    Integer {
        #[serde(with = "int64")]
        value: i64,
    },
    #[serde(rename = "optionValueString")]
    String { value: String },
}

/// TDLib's JSON writes int64 values as strings: they do not fit in a JavaScript number.
mod int64 {
    use serde::{Deserialize, Deserializer};

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<i64, D::Error> {
        String::deserialize(deserializer)?.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn int64_values_are_read_from_strings() {
        let value: OptionValue =
            serde_json::from_str(r#"{"@type":"optionValueInteger","value":"9007199254740993"}"#).unwrap();
        assert_eq!(value, OptionValue::Integer { value: 9_007_199_254_740_993 });
    }
}
