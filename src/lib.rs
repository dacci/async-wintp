mod util;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Win32(#[from] threadpool::Error),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

pub mod runtime;

pub mod task;
pub use task::spawn;

#[cfg(feature = "time")]
pub mod time;

#[cfg(feature = "io")]
pub mod io;

#[cfg(feature = "fs")]
pub mod fs;
