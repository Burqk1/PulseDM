use std::sync::atomic::{AtomicBool, Ordering};

static ENGLISH: AtomicBool = AtomicBool::new(true);

pub fn set_english(on: bool) {
    ENGLISH.store(on, Ordering::Relaxed);
}

pub fn english() -> bool {
    ENGLISH.load(Ordering::Relaxed)
}

pub fn system_turkish() -> bool {
    sys_locale::get_locale().is_some_and(|l| l.to_ascii_lowercase().starts_with("tr"))
}

macro_rules! text {
    ($en:literal, $tr:literal $(,)?) => {
        if $crate::lang::english() { format!($en) } else { format!($tr) }
    };
}
pub(crate) use text;
