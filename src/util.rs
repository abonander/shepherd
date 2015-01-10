use std::fmt;
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

impl fmt::String for FormatBytes {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> Result<(), fmt::Error> {
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

pub fn precise_time_ms() -> u64 {
    use time;
    time::precise_time_ns() / 1_000_000    
}

pub struct FormatTime(pub u64, pub u8, pub u8);

impl FormatTime {
    pub fn from_s(s: u64) -> FormatTime {
        let seconds = (s % 60) as u8;
        let tot_min = s / 60;
        let minutes = (tot_min % 60) as u8;
        let hours = tot_min / 60;
        
        FormatTime(hours, minutes, seconds)               
    }   
}

impl fmt::String for FormatTime {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        f.write_fmt(format_args!("{}:{:02}:{:02}", self.0, self.1, self.2))    
    }    
}
