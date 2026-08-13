static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::time::Duration;

const TICK_INTERVAL_SECS: u64 = 5;

fn main() {
    println!("[cue_daemon] запущен, pid={}", std::process::id());

    let mut tick: u64 = 0;
    loop {
        tick += 1;
        println!("[cue_daemon] tick #{tick}");
        std::thread::sleep(Duration::from_secs(TICK_INTERVAL_SECS));
    }
}