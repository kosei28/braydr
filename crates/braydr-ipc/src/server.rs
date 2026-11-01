//! The Unix socket the app listens on for `braydr-mcp`.

use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::{Arc, Mutex};

use braydr_store::Store;

use crate::{Request, Response, tools, views};

/// Binds with owner-only permissions, replacing a socket no one is listening on.
pub fn bind(path: &Path) -> io::Result<UnixListener> {
    if UnixStream::connect(path).is_err() && path.exists() {
        std::fs::remove_file(path)?;
    }
    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

pub fn handle(store: &mut Store, request: &Request) -> Response {
    match tools::call(
        store,
        request.channel.as_deref(),
        &request.tool,
        &request.args,
    ) {
        Ok(value) => Response::Result(value),
        Err(error) => Response::Error(views::error(&error)),
    }
}

pub fn serve(listener: UnixListener, store: Arc<Mutex<Store>>) {
    for stream in listener.incoming().flatten() {
        let store = store.clone();
        std::thread::spawn(move || {
            let _ = connection(stream, &store);
        });
    }
}

fn connection(stream: UnixStream, store: &Mutex<Store>) -> io::Result<()> {
    let mut writer = stream.try_clone()?;
    for line in BufReader::new(stream).lines() {
        let line = line?;
        let response = match serde_json::from_str::<Request>(&line) {
            Ok(request) => handle(
                &mut store.lock().expect("the store lock is not poisoned"),
                &request,
            ),
            Err(error) => Response::Error(views::error(&braydr_core::Error::InvalidArgument {
                message: error.to_string(),
            })),
        };
        serde_json::to_writer(&mut writer, &response)?;
        writer.write_all(b"\n")?;
    }
    Ok(())
}
