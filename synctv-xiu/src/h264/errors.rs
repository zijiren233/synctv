use crate::bytesio::bits_errors::BitError;

#[derive(Debug, thiserror::Error)]
pub enum H264ErrorValue {
    #[error("bit error")]
    BitError(BitError),
}

#[derive(Debug, thiserror::Error)]
#[error("{value}")]
pub struct H264Error {
    pub value: H264ErrorValue,
}

impl From<BitError> for H264Error {
    fn from(error: BitError) -> Self {
        Self {
            value: H264ErrorValue::BitError(error),
        }
    }
}
