#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod control_server;
mod cue_liveness;
mod icon_cache;
mod manifest;
mod notify;
mod project;
mod routine_scheduler;

pub(crate) static ICON_PNG: &[u8] = include_bytes!("../icon.png");

pub fn app_dir() -> std::path::PathBuf {
    #[cfg(target_os = "windows")]
    {
        std::env::var("APPDATA")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::path::PathBuf::from("."))
            .join("Cue")
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| {
                std::env::var("HOME")
                    .map(|h| std::path::PathBuf::from(h).join(".config"))
                    .unwrap_or_else(|_| std::path::PathBuf::from("."))
            })
            .join("cue")
    }
}

const TICK_INTERVAL_SECS: u64 = 60;

fn tick(projects: &mut [project::LoadedProject]) {
    if cue_liveness::cue_is_running() {
        println!("[cue_daemon] Cue жива, пропускаю тик");
        return;
    }

    let now = routine_scheduler::local_now();

    for p in projects.iter_mut() {
        let mut changed = false;

        for (id, task) in p.main.iter_mut().chain(p.subs.iter_mut()) {
            let Some(routine) = task.routine.as_mut() else { continue };
            if routine.active { continue }
            let Some(occ) = routine_scheduler::due_occurrence(routine, now) else { continue };

            routine.active = true;
            routine.last_triggered_at = now;
            routine_scheduler::prune_expired_direct(routine, now);
            changed = true;

            println!(
                "  ACTIVATED: [{}] {} — задача '{}' (task_id={id}), occ={occ}",
                p.name, p.color_hex, task.text
            );

            if now.saturating_sub(occ) <= routine_scheduler::NOTIFY_WINDOW_SECS {
                notify::send(&task.text, &p.name, &p.color_hex);
            }
        }

        if changed {
            p.save(now);
            println!("[cue_daemon] сохранён проект {} ({})", p.name, p.id);
        }
    }
}

fn main() {
    println!("[cue_daemon] запущен, pid={}", std::process::id());

    control_server::start();

    let mut projects = project::load_all_projects();
    println!("[cue_daemon] прочитано проектов: {}", projects.len());

    loop {
        tick(&mut projects);
        std::thread::sleep(std::time::Duration::from_secs(TICK_INTERVAL_SECS));
    }
}
