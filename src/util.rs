use std::fmt::{self, Show, Formatter};
use std::io::{IoErrorKind, IoResult};

#[derive(Copy)]
pub struct FormatBytes(pub u64);

impl FormatBytes { 
    #[inline]
    fn to_kb(self) -> f64 {
        (self.0 as f64) / 1.0e3   
    }

    #[inline]
    fn to_mb(self) -> f64 {
        (self.0 as f64) / 1.0e6
    }

    #[inline]
    fn to_gb(self) -> f64 {
        (self.0 as f64) / 1.0e9
    }
}

impl Show for FormatBytes {
    fn fmt(&self, fmt: &mut Formatter) -> Result<(), fmt::Error> {
        match self.0 {
            0 ... 999 => fmt.write_fmt(format_args!("{} B", self.0)),
            1_000 ... 999_999 => fmt.write_fmt(format_args!("{:.02} KB", self.to_kb())),
            1_000_000 ... 999_999_999 => fmt.write_fmt(format_args!("{:.02} MB", self.to_mb())),
            _ => fmt.write_fmt(format_args!("{:.02} GB", self.to_gb())),
        }
    }
}

pub fn ignore_timeout<T>(result: IoResult<T>) -> IoResult<Option<T>> {
    match result {
        Ok(ok) => Ok(Some(ok)),
        Err(ref err) if err.kind == IoErrorKind::TimedOut => Ok(None),
        Err(err) => Err(err),
    }
}

