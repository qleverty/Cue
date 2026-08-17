use tiny_http::{Method, Response, Server, StatusCode};

use crate::exclusive_bind::bind_exclusive;

pub const PORT: u16 = 52684;

pub fn start() {
    std::thread::Builder::new()
        .name("cue-daemon-control".into())
        .spawn(run)
        .expect("spawn control server thread");
}

const BIND_RETRY_MS:       u64 = 300;
const RELEASE_COOLDOWN_MS: u64 = 1000;

fn run() {
    loop {
        let server = bind_with_retry();
        println!("[cue_daemon] control-сервер слушает порт {PORT}");
        serve(&server);
        drop(server);
        println!("[cue_daemon] control-сервер отпустил порт {PORT}, жду {RELEASE_COOLDOWN_MS}мс перед повтором");
        std::thread::sleep(std::time::Duration::from_millis(RELEASE_COOLDOWN_MS));
    }
}

fn bind_with_retry() -> Server {
    let mut logged_wait = false;
    loop {
        let attempt = bind_exclusive(PORT).ok()
            .and_then(|l| Server::from_listener(l, None).ok());
        match attempt {
            Some(server) => return server,
            None => {
                if !logged_wait {
                    println!("[cue_daemon] порт {PORT} занят, жду освобождения");
                    logged_wait = true;
                }
                std::thread::sleep(std::time::Duration::from_millis(BIND_RETRY_MS));
            }
        }
    }
}

fn serve(server: &Server) {
    for req in server.incoming_requests() {
        let raw = req.url().to_owned();
        let (path, query) = raw.split_once('?').unwrap_or((&raw, ""));
        let is_loopback = req.remote_addr()
            .map(|a| a.ip().is_loopback())
            .unwrap_or(false);

        if req.method() == &Method::Get && path == "/1/control" && query.contains("cmd=yield") {
            if !is_loopback {
                println!("[cue_daemon] отклонён yield не с loopback: {:?}", req.remote_addr());
                let _ = req.respond(Response::from_string("").with_status_code(StatusCode(403)));
                continue;
            }

            println!("[cue_daemon] получен yield от {:?}, отпускаю порт", req.remote_addr());
            let _ = req.respond(Response::from_string("").with_status_code(StatusCode(200)));
            server.unblock();
        } else {
            let _ = req.respond(Response::from_string("").with_status_code(StatusCode(404)));
        }
    }
}
