//! Keeps Windows from sleeping while SillyTavern is running, so a phone can keep using it.
//! `SetThreadExecutionState` is per thread, so a long-lived worker thread owns the flag.

use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, OnceLock};

use windows::Win32::System::Power::{
    SetThreadExecutionState, ES_CONTINUOUS, ES_SYSTEM_REQUIRED,
};

static TX: OnceLock<Mutex<Sender<bool>>> = OnceLock::new();

/// `true`: the system stays awake (the display may still turn off). `false`: normal.
pub fn set(keep_awake: bool) {
    let tx = TX.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<bool>();
        std::thread::Builder::new()
            .name("keep-awake".into())
            .spawn(move || {
                let mut current = false;
                while let Ok(keep) = rx.recv() {
                    if keep == current {
                        continue;
                    }
                    current = keep;
                    unsafe {
                        let flags = if keep {
                            ES_CONTINUOUS | ES_SYSTEM_REQUIRED
                        } else {
                            ES_CONTINUOUS
                        };
                        SetThreadExecutionState(flags);
                    }
                    log::info!("keep-awake {}", if keep { "on" } else { "off" });
                }
            })
            .expect("keep-awake thread");
        Mutex::new(tx)
    });
    if let Ok(tx) = tx.lock() {
        let _ = tx.send(keep_awake);
    }
}
