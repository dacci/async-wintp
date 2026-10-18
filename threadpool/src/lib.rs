mod util;

pub use windows_result::{Error, Result};

pub mod pool;
pub use pool::Pool;

pub mod work;
pub use work::PoolWork;

#[cfg(feature = "timer")]
pub mod timer;
#[cfg(feature = "timer")]
pub use timer::PoolTimer;

#[cfg(feature = "io")]
pub mod io;
#[cfg(feature = "io")]
pub use io::PoolIo;

#[cfg(feature = "sync")]
pub mod sync;
#[cfg(feature = "sync")]
pub use sync::PoolWait;
