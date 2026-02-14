use std::io;
// use tokio::time::Elapsed;

#[derive(Debug, thiserror::Error)]
pub enum BytesIOErrorValue {
    #[error("not enough bytes")]
    NotEnoughBytes,
    #[error("empty stream")]
    EmptyStream,
    #[error("io error")]
    IOError(io::Error),
    #[error("time out error")]
    TimeoutError(tokio::time::error::Elapsed),
    #[error("none return")]
    NoneReturn,
}

#[derive(Debug, thiserror::Error)]
#[error("{value}")]
pub struct BytesIOError {
    pub value: BytesIOErrorValue,
}

impl From<BytesIOErrorValue> for BytesIOError {
    fn from(val: BytesIOErrorValue) -> Self {
        Self { value: val }
    }
}

impl From<io::Error> for BytesIOError {
    fn from(error: io::Error) -> Self {
        Self {
            value: BytesIOErrorValue::IOError(error),
        }
    }
}

// impl From<Elapsed> for NetIOError {
//     fn from(error: Elapsed) -> Self {
//         NetIOError {
//             value: NetIOErrorValue::TimeoutError(error),
//         }
//     }
// }
