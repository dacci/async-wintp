use std::time::Duration;
use windows_sys::Win32::Foundation::FILETIME;

#[allow(unused)]
pub trait AsFILETIME {
    fn as_filetime(&self) -> FILETIME;
}

impl AsFILETIME for Duration {
    fn as_filetime(&self) -> FILETIME {
        let dur = -((self.as_nanos() / 100) as i64);

        FILETIME {
            dwLowDateTime: dur as _,
            dwHighDateTime: (dur >> 32) as _,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_as_filetime() {
        let ft = Duration::from_nanos(100).as_filetime();
        assert_eq!(ft.dwLowDateTime, 0xFFFFFFFF);
        assert_eq!(ft.dwHighDateTime, 0xFFFFFFFF);

        let ft = Duration::from_nanos(0).as_filetime();
        assert_eq!(ft.dwLowDateTime, 0);
        assert_eq!(ft.dwHighDateTime, 0);
    }
}
