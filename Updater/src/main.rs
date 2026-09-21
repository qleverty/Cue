#![windows_subsystem = "windows"]

use std::{env, fs, thread, time::Duration};

fn main() {
    for arg in env::args().skip(1) {
        let Some(target) = arg.strip_suffix(".cueextraupd") else { continue };
        if fs::rename(&arg, target).is_ok() { continue; }
        for wait in [3, 6, 9] {
            thread::sleep(Duration::from_secs(wait));
            if fs::rename(&arg, target).is_ok() { break; }
        }
    }
}
