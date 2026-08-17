use tiny_http::{Method, Response, Server, StatusCode};

pub const PORT: u16 = 52684;

pub fn start() {
    std::thread::Builder::new()
        .name("cue-daemon-control".into())
        .spawn(run)
        .expect("spawn control server thread");
}

fn run() {
    let server = match Server::http(format!("0.0.0.0:{PORT}")) {
        Ok(s)  => s,
        Err(e) => { println!("[cue_daemon] не удалось забиндить порт {PORT}: {e}"); return; }
    };
    println!("[cue_daemon] control-сервер слушает порт {PORT}");

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

    println!("[cue_daemon] control-сервер отпустил порт {PORT}, поток завершён");
}
