#![allow(non_local_definitions)]
use {
    crate::bytesio::bytes_errors::{BytesReadError, BytesWriteError},
    failure::{Backtrace, Fail},
    std::{fmt, io::Error, time::SystemTimeError},
};

#[derive(Debug, Fail)]
pub enum HandshakeErrorValue {
    #[fail(display = "bytes read error: {}", _0)]
    BytesReadError(BytesReadError),
    #[fail(display = "bytes write error: {}", _0)]
    BytesWriteError(BytesWriteError),
    #[fail(display = "system time error: {}", _0)]
    SysTimeError(SystemTimeError),
    #[fail(display = "digest error: {}", _0)]
    DigestError(DigestError),
    #[fail(display = "Digest not found error")]
    DigestNotFound,
    #[fail(display = "s0 version not correct error")]
    S0VersionNotCorrect,
    #[fail(display = "io error")]
    IOError(Error),
}

impl From<Error> for HandshakeError {
    fn from(error: Error) -> Self {
        Self {
            value: HandshakeErrorValue::IOError(error),
        }
    }
}

#[derive(Debug)]
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

impl fmt::Display for HandshakeError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(&self.value, f)
    }
}

impl Fail for HandshakeError {
    fn cause(&self) -> Option<&dyn Fail> {
        self.value.cause()
    }

    fn backtrace(&self) -> Option<&Backtrace> {
        self.value.backtrace()
    }
}

#[derive(Debug)]
pub struct DigestError {
    pub value: DigestErrorValue,
}

#[derive(Debug, Fail)]
pub enum DigestErrorValue {
    #[fail(display = "bytes read error: {}", _0)]
    BytesReadError(BytesReadError),
    #[fail(display = "digest length not correct")]
    DigestLengthNotCorrect,
    #[fail(display = "cannot generate digest")]
    CannotGenerate,
    #[fail(display = "unknow schema")]
    UnknowSchema,
    #[fail(display = "HMAC key initialization failed")]
    HmacInitError,
}

impl From<BytesReadError> for DigestError {
    fn from(error: BytesReadError) -> Self {
        Self {
            value: DigestErrorValue::BytesReadError(error),
        }
    }
}

impl fmt::Display for DigestError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(&self.value, f)
    }
}

impl Fail for DigestError {
    fn cause(&self) -> Option<&dyn Fail> {
        self.value.cause()
    }

    fn backtrace(&self) -> Option<&Backtrace> {
        self.value.backtrace()
    }
}
