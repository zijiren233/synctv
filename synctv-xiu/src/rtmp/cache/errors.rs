#![allow(non_local_definitions)]
use {
    crate::rtmp::chunk::errors::PackError,
    crate::bytesio::bytes_errors::BytesReadError,
    failure::{Backtrace, Fail},
    crate::h264::errors::H264Error,
    std::fmt,
    crate::flv::amf0::errors::Amf0WriteError,
    crate::flv::errors::{FlvDemuxerError, Mpeg4AvcHevcError, MpegAacError},
};

#[derive(Debug, Fail)]
pub enum CacheErrorValue {
    #[fail(display = "cache tag parse error")]
    DemuxerError(FlvDemuxerError),
    #[fail(display = "mpeg aac error")]
    MpegAacError(MpegAacError),
    #[fail(display = "mpeg avc error")]
    MpegAvcError(Mpeg4AvcHevcError),
    #[fail(display = "pack error")]
    PackError(PackError),
    #[fail(display = "read bytes error")]
    BytesReadError(BytesReadError),
    #[fail(display = "h264 error")]
    H264Error(H264Error),
}

impl fmt::Display for CacheError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(&self.value, f)
    }
}
#[derive(Debug)]
pub struct CacheError {
    pub value: CacheErrorValue,
}

impl From<FlvDemuxerError> for CacheError {
    fn from(error: FlvDemuxerError) -> Self {
        Self {
            value: CacheErrorValue::DemuxerError(error),
        }
    }
}

impl From<H264Error> for CacheError {
    fn from(error: H264Error) -> Self {
        Self {
            value: CacheErrorValue::H264Error(error),
        }
    }
}

impl From<MpegAacError> for CacheError {
    fn from(error: MpegAacError) -> Self {
        Self {
            value: CacheErrorValue::MpegAacError(error),
        }
    }
}

impl From<Mpeg4AvcHevcError> for CacheError {
    fn from(error: Mpeg4AvcHevcError) -> Self {
        Self {
            value: CacheErrorValue::MpegAvcError(error),
        }
    }
}

impl From<BytesReadError> for CacheError {
    fn from(error: BytesReadError) -> Self {
        Self {
            value: CacheErrorValue::BytesReadError(error),
        }
    }
}

impl From<PackError> for CacheError {
    fn from(error: PackError) -> Self {
        Self {
            value: CacheErrorValue::PackError(error),
        }
    }
}

impl Fail for CacheError {
    fn cause(&self) -> Option<&dyn Fail> {
        self.value.cause()
    }

    fn backtrace(&self) -> Option<&Backtrace> {
        self.value.backtrace()
    }
}

#[derive(Debug, Fail)]
pub enum MetadataErrorValue {
    #[fail(display = "metadata tag parse error")]
    DemuxerError(FlvDemuxerError),
    #[fail(display = "pack error")]
    PackError(PackError),
    #[fail(display = "amf write error")]
    Amf0WriteError(Amf0WriteError),
}
#[derive(Debug)]
pub struct MetadataError {
    pub value: MetadataErrorValue,
}

impl From<Amf0WriteError> for MetadataError {
    fn from(error: Amf0WriteError) -> Self {
        Self {
            value: MetadataErrorValue::Amf0WriteError(error),
        }
    }
}

impl Fail for MetadataError {
    fn cause(&self) -> Option<&dyn Fail> {
        self.value.cause()
    }

    fn backtrace(&self) -> Option<&Backtrace> {
        self.value.backtrace()
    }
}

impl fmt::Display for MetadataError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(&self.value, f)
    }
}
