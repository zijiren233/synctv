#![allow(non_local_definitions)]
use crate::bytesio::bytes_errors::BytesReadError;
use crate::bytesio::bytes_errors::BytesWriteError;
use failure::Backtrace;
use serde_json::error::Error;
use tokio::sync::oneshot::error::RecvError;

use {failure::Fail, std::fmt};
#[derive(Debug, Fail)]
pub enum StreamHubErrorValue {
    #[fail(display = "no app name")]
    NoAppName,
    #[fail(display = "no stream name")]
    NoStreamName,
    #[fail(display = "no app or stream name")]
    NoAppOrStreamName,
    #[fail(display = "exists")]
    Exists,
    #[fail(display = "send error")]
    SendError,
    #[fail(display = "send video error")]
    SendVideoError,
    #[fail(display = "send audio error")]
    SendAudioError,
    #[fail(display = "bytes read error")]
    BytesReadError(BytesReadError),
    #[fail(display = "bytes write error")]
    BytesWriteError(BytesWriteError),
    #[fail(display = "not correct data sender type")]
    NotCorrectDataSenderType,
    #[fail(display = "subscriber channel closed")]
    SubscriberClosed,
    #[fail(display = "Tokio oneshot recv error")]
    RecvError(RecvError),
    #[fail(display = "Serde json error")]
    SerdeError(Error),
    #[fail(display = "client session error: {}", _0)]
    ClientSessionError(String),
}
#[derive(Debug)]
pub struct StreamHubError {
    pub value: StreamHubErrorValue,
}

impl fmt::Display for StreamHubError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(&self.value, f)
    }
}

impl Fail for StreamHubError {
    fn cause(&self) -> Option<&dyn Fail> {
        self.value.cause()
    }

    fn backtrace(&self) -> Option<&Backtrace> {
        self.value.backtrace()
    }
}

impl From<BytesReadError> for StreamHubError {
    fn from(error: BytesReadError) -> Self {
        Self {
            value: StreamHubErrorValue::BytesReadError(error),
        }
    }
}

impl From<BytesWriteError> for StreamHubError {
    fn from(error: BytesWriteError) -> Self {
        Self {
            value: StreamHubErrorValue::BytesWriteError(error),
        }
    }
}

impl From<RecvError> for StreamHubError {
    fn from(error: RecvError) -> Self {
        Self {
            value: StreamHubErrorValue::RecvError(error),
        }
    }
}

impl From<Error> for StreamHubError {
    fn from(error: Error) -> Self {
        Self {
            value: StreamHubErrorValue::SerdeError(error),
        }
    }
}

impl From<String> for StreamHubError {
    fn from(error: String) -> Self {
        Self {
            value: StreamHubErrorValue::ClientSessionError(error),
        }
    }
}

// impl From<CacheError> for ChannelError {
//     fn from(error: CacheError) -> Self {
//         ChannelError {
//             value: ChannelErrorValue::CacheError(error),
//         }
//     }
// }
