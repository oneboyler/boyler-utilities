/// Every error the Timers features return. Nothing in this crate panics on purpose.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TimerError {
    /// The typed time can't be read (or is zero).
    #[error("not a time: {0:?}")]
    InvalidTime(String),
    /// The countdown's time can't be typed while it runs (the drawing makes the digits read-only).
    #[error("the countdown is running")]
    Running,
    /// No bar with this id (it was removed).
    #[error("no timer bar {0}")]
    NoSuchBar(u32),
    /// The on-screen timer bars card is switched off: bars don't start.
    #[error("on-screen timer bars are off")]
    BarsOff,
    /// Playing the chime failed in Windows.
    #[error("{context} failed (0x{code:08X})")]
    Os { context: String, code: u32 },
}

pub type Result<T> = std::result::Result<T, TimerError>;
