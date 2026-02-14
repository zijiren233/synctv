use {
    crate::bytesio::bytes_errors::{BytesReadError, BytesWriteError},
    std::{io::Error, time::SystemTimeError},
};

#[derive(Debug, thiserror::Error)]
pub enum HandshakeErrorValue {
    #[error("bytes read error: {0}")]
    BytesReadError(BytesReadError),
    #[error("bytes write error: {0}")]
    BytesWriteError(BytesWriteError),
    #[error("system time error: {0}")]
    SysTimeError(SystemTimeError),
    #[error("digest error: {0}")]
    DigestError(DigestError),
    #[error("Digest not found error")]
    DigestNotFound,
    #[error("s0 version not correct error")]
    S0VersionNotCorrect,
    #[error("io error")]
    IOError(Error),
}

impl From<Error> for HandshakeError {
    fn from(error: Error) -> Self {
        Self {
            value: HandshakeErrorValue::IOError(error),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{value}")]
pub struct HandshakeError {
    pub value: HandshakeErrorValue,
}

impl From<HandshakeErrorValue> for HandshakeError {
    fn from(val: HandshakeErrorValue) -> Self {
        Self { value: val }
    }
}

impl From<BytesReadError> for HandshakeError {
    fn from(error: BytesReadError) -> Self {
        Self {
            value: HandshakeErrorValue::BytesReadError(error),
        }
    }
}

impl From<BytesWriteError> for HandshakeError {
    fn from(error: BytesWriteError) -> Self {
        Self {
            value: HandshakeErrorValue::BytesWriteError(error),
        }
    }
}

impl From<SystemTimeError> for HandshakeError {
    fn from(error: SystemTimeError) -> Self {
        Self {
            value: HandshakeErrorValue::SysTimeError(error),
        }
    }
}

impl From<DigestError> for HandshakeError {
    fn from(error: DigestError) -> Self {
        Self {
            value: HandshakeErrorValue::DigestError(error),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{value}")]
pub struct DigestError {
    pub value: DigestErrorValue,
}

#[derive(Debug, thiserror::Error)]
pub enum DigestErrorValue {
    #[error("bytes read error: {0}")]
    BytesReadError(BytesReadError),
    #[error("digest length not correct")]
    DigestLengthNotCorrect,
    #[error("cannot generate digest")]
    CannotGenerate,
    #[error("unknow schema")]
    UnknowSchema,
    #[error("HMAC key initialization failed")]
    HmacInitError,
}

impl From<BytesReadError> for DigestError {
    fn from(error: BytesReadError) -> Self {
        Self {
            value: DigestErrorValue::BytesReadError(error),
        }
    }
}
