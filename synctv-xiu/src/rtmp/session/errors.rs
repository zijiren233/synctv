use {
    crate::rtmp::{
        cache::errors::CacheError,
        chunk::errors::{PackError, UnpackError},
        handshake::errors::HandshakeError,
        messages::errors::MessageError,
        netconnection::errors::NetConnectionError,
        netstream::errors::NetStreamError,
        protocol_control_messages::errors::ControlMessagesError,
        user_control_messages::errors::EventMessagesError,
    },
    crate::bytesio::{bytes_errors::BytesWriteError, bytesio_errors::BytesIOError},
    crate::streamhub::errors::StreamHubError,
    tokio::sync::oneshot::error::RecvError,
    crate::flv::amf0::errors::Amf0WriteError,
};

#[derive(Debug, thiserror::Error)]
#[error("{value}")]
pub struct SessionError {
    pub value: SessionErrorValue,
}

#[derive(Debug, thiserror::Error)]
pub enum SessionErrorValue {
    #[error("amf0 write error: {0}")]
    Amf0WriteError(#[source] Amf0WriteError),
    #[error("bytes write error: {0}")]
    BytesWriteError(#[source] BytesWriteError),
    #[error("unpack error: {0}")]
    UnPackError(#[source] UnpackError),

    #[error("message error: {0}")]
    MessageError(#[source] MessageError),
    #[error("control message error: {0}")]
    ControlMessagesError(#[source] ControlMessagesError),
    #[error("net connection error: {0}")]
    NetConnectionError(#[source] NetConnectionError),
    #[error("net stream error: {0}")]
    NetStreamError(#[source] NetStreamError),

    #[error("event messages error: {0}")]
    EventMessagesError(#[source] EventMessagesError),
    #[error("net io error: {0}")]
    BytesIOError(#[source] BytesIOError),
    #[error("pack error: {0}")]
    PackError(#[source] PackError),
    #[error("handshake error: {0}")]
    HandshakeError(#[source] HandshakeError),
    #[error("cache error name: {0}")]
    CacheError(#[source] CacheError),
    #[error("tokio: oneshot receiver err: {0}")]
    RecvError(#[source] RecvError),
    #[error("streamhub channel err: {0}")]
    ChannelError(#[source] StreamHubError),

    #[error("amf0 count not correct error")]
    Amf0ValueCountNotCorrect,
    #[error("amf0 value type not correct error")]
    Amf0ValueTypeNotCorrect,
    #[error("stream hub event send error")]
    StreamHubEventSendErr,
    #[error("none frame data sender error")]
    NoneFrameDataSender,
    #[error("none frame data receiver error")]
    NoneFrameDataReceiver,
    #[error("send frame data error")]
    SendFrameDataErr,
    #[error("subscribe count limit is reached.")]
    SubscribeCountLimitReach,

    #[error("no app name error")]
    NoAppName,
    #[error("no stream name error")]
    NoStreamName,
    #[error("no media data can be received now.")]
    NoMediaDataReceived,

    #[error("session is finished.")]
    Finish,
    #[error("auth failed: {0}")]
    AuthFailed(String),
    #[error("handshake timeout")]
    Timeout,
}

impl From<Amf0WriteError> for SessionError {
    fn from(error: Amf0WriteError) -> Self {
        Self {
            value: SessionErrorValue::Amf0WriteError(error),
        }
    }
}

impl From<BytesWriteError> for SessionError {
    fn from(error: BytesWriteError) -> Self {
        Self {
            value: SessionErrorValue::BytesWriteError(error),
        }
    }
}

impl From<UnpackError> for SessionError {
    fn from(error: UnpackError) -> Self {
        Self {
            value: SessionErrorValue::UnPackError(error),
        }
    }
}

impl From<MessageError> for SessionError {
    fn from(error: MessageError) -> Self {
        Self {
            value: SessionErrorValue::MessageError(error),
        }
    }
}

impl From<ControlMessagesError> for SessionError {
    fn from(error: ControlMessagesError) -> Self {
        Self {
            value: SessionErrorValue::ControlMessagesError(error),
        }
    }
}

impl From<NetConnectionError> for SessionError {
    fn from(error: NetConnectionError) -> Self {
        Self {
            value: SessionErrorValue::NetConnectionError(error),
        }
    }
}

impl From<NetStreamError> for SessionError {
    fn from(error: NetStreamError) -> Self {
        Self {
            value: SessionErrorValue::NetStreamError(error),
        }
    }
}

impl From<EventMessagesError> for SessionError {
    fn from(error: EventMessagesError) -> Self {
        Self {
            value: SessionErrorValue::EventMessagesError(error),
        }
    }
}

impl From<BytesIOError> for SessionError {
    fn from(error: BytesIOError) -> Self {
        Self {
            value: SessionErrorValue::BytesIOError(error),
        }
    }
}

impl From<PackError> for SessionError {
    fn from(error: PackError) -> Self {
        Self {
            value: SessionErrorValue::PackError(error),
        }
    }
}

impl From<HandshakeError> for SessionError {
    fn from(error: HandshakeError) -> Self {
        Self {
            value: SessionErrorValue::HandshakeError(error),
        }
    }
}

impl From<CacheError> for SessionError {
    fn from(error: CacheError) -> Self {
        Self {
            value: SessionErrorValue::CacheError(error),
        }
    }
}

impl From<RecvError> for SessionError {
    fn from(error: RecvError) -> Self {
        Self {
            value: SessionErrorValue::RecvError(error),
        }
    }
}

impl From<StreamHubError> for SessionError {
    fn from(error: StreamHubError) -> Self {
        Self {
            value: SessionErrorValue::ChannelError(error),
        }
    }
}
