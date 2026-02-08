use anyhow::{Result, anyhow, bail};
use cpp::cpp;
use dirs::runtime_dir;
use log::{debug, error, info, warn};
use qmetaobject::QObjectPinned;
use qmetaobject::prelude::*;
use qmetaobject::webengine;
use std::cell::RefCell;
use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;
use std::{env, fs, thread};
use tungstenite::http::Uri;
use tungstenite::{Message, connect};

mod window_handler;
mod window_properties;

use crate::window_handler::{WindowHandler, WindowMessage};
use window_properties::WindowProperties;

const APP_NAME: &str = "pipeweaver-app";

cpp! {{
    #include <QGuiApplication>
    #include <QIcon>
    #include <QString>
}}

qrc!(pipeweaver_resources,
    "webengine" {
        "main.qml",
        "resources/pipeweaver.svg",
    },
);

fn main() -> Result<()> {
    if let Err(e) = real_main() {
        display_error(format!("{e}"));
        bail!(e);
    }

    Ok(())
}

fn real_main() -> Result<()> {
    unsafe {
        //env::set_var("QT_QPA_PLATFORM", "xcb");
        env::set_var("RUST_LOG", "debug");
        env::set_var(
            "QTWEBENGINE_CHROMIUM_FLAGS",
            "  --enable-features=Canvas2DImageChromium \
                     --enable-gpu-memory-buffer-compositor-resources \
                     --enable-zero-copy \
                     --force-gpu-mem-available-mb=256 \
                     --max-decoded-image-size-mb=64 \
                     --js-flags=--expose-gc,--max-old-space-size=128 \
                     --disable-software-rasterizer \
                     --disable-dev-shm-usage \
                     --disable-gpu-shader-disk-cache \
                     --num-raster-threads=2 \
                     --single-process",
        );
    }
    env_logger::init();

    if handle_active_instance() {
        println!("Instance Already active, Exiting");
        return Ok(());
    }

    // Channel for notifications from code to the Window
    let (notify_tx, notify_rx) = mpsc::channel();

    // Ok, lets try getting the websocket running
    let (res_tx, res_rx) = mpsc::channel();
    let notify_websocket = notify_tx.clone();
    thread::spawn(move || {
        websocket_main_thread(res_tx, notify_websocket);
    });

    if let Err(e) = res_rx.recv()? {
        error!("Failed to Connect to Pipeweaver: {e}");
        bail!("Cannot Start, Pipeweaver is not running.   ");
    }

    webengine::initialize();
    pipeweaver_resources();

    // Configure QT to pick the relevant desktop file
    unsafe {
        cpp!([] {
            QGuiApplication::setDesktopFileName("pipeweaver-app");
            QGuiApplication::setWindowIcon(QIcon(QString(":/webengine/resources/pipeweaver.svg")));
        });
    }

    // Spawn the IPC thread with only the sender (thread must NOT touch QObjects)
    thread::spawn(move || {
        if let Err(e) = ipc_thread_main(notify_tx) {
            warn!("IPC thread exited with error: {e}");
        }
    });

    // Create the engine and link up the rust side
    let mut engine = QmlEngine::new();

    let window_props = Rc::new(RefCell::new(WindowProperties::new()));
    let ipc_handler = Rc::new(RefCell::new(WindowHandler::new(notify_rx)));
    unsafe {
        engine.set_object_property(
            "windowProperties".into(),
            QObjectPinned::new(window_props.as_ref()),
        );

        engine.set_object_property(
            "windowHandler".into(),
            QObjectPinned::new(ipc_handler.as_ref()),
        );
    }

    engine.load_file("qrc:/webengine/main.qml".into());
    engine.exec();

    Ok(())
}

fn websocket_main_thread(res: mpsc::Sender<Result<()>>, tx: mpsc::Sender<WindowMessage>) {
    // We need to spawn up a Websocket connection, then simply read from it until closed
    let uri = match Uri::builder()
        .authority("localhost:14565")
        .scheme("ws")
        .path_and_query("/api/websocket")
        .build()
    {
        Ok(uri) => uri,
        Err(e) => {
            let _ = res.send(Err(anyhow!(e)));
            return;
        }
    };

    info!("Attempting to connect to Pipeweaver at {uri}");
    let (mut socket, response) = match connect(uri) {
        Ok((socket, response)) => (socket, response),
        Err(e) => {
            let _ = res.send(Err(anyhow!(e)));
            return;
        }
    };

    info!("Connected, HTTP status: {}", response.status());
    let _ = res.send(Ok(()));

    loop {
        match socket.read() {
            Ok(msg) => {
                // NOOP everything except Ping/Pong
                match msg {
                    Message::Ping(payload) => {
                        let _ = socket.send(Message::Pong(payload));
                    }
                    Message::Close(_) => {
                        println!("Server closed the connection");
                        break;
                    }
                    _ => {}
                }
            }
            Err(tungstenite::Error::ConnectionClosed) => {
                error!("Disconnected: connection closed");
                break;
            }
            Err(tungstenite::Error::Protocol(e)) => {
                error!("Disconnected: protocol error: {e}");
                break;
            }
            Err(e) => {
                error!("Disconnected: other error: {e}");

                break;
            }
        }
    }

    // If we get here, the connection has been dropped, close our window.
    info!("Connection to Pipeweaver Lost, sending Close");
    let _ = tx.send(WindowMessage::Close);
}

fn ipc_thread_main(tx: mpsc::Sender<WindowMessage>) -> Result<()> {
    debug!("Spawning IPC Socket Handler");

    let socket_path = get_socket_file_path();
    if let Some(parent) = socket_path.parent()
        && let Err(e) = fs::create_dir_all(parent)
    {
        warn!("Failed to create socket directory {parent:?}: {e}");
        bail!("Failed to Open IPC Socket");
    }

    if socket_path.exists() {
        let _ = fs::remove_file(&socket_path);
    }

    let listener = match UnixListener::bind(&socket_path) {
        Ok(listener) => listener,
        Err(e) => {
            warn!("Failed to bind to socket: {e}");
            bail!("Failed to bind to socket: {e}");
        }
    };

    listener.set_nonblocking(true)?;

    debug!("IPC listener started at {socket_path:?}");
    loop {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let mut msg = String::new();
                if let Err(e) = stream.read_to_string(&mut msg) {
                    warn!("Failed to read message from stream: {e}");
                } else if msg == "TRIGGER" {
                    let _ = tx.send(WindowMessage::Trigger);
                }
            }
            Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => {
                warn!("Unexpected socket error: {e}");
                break;
            }
        }
    }
    let _ = fs::remove_file(&socket_path);
    debug!("IPC Socket closed (thread)");
    Ok(())
}

pub fn handle_active_instance() -> bool {
    let socket_path = get_socket_file_path();
    debug!("Looking for Socket at {socket_path:?}");

    if !socket_path.exists() {
        debug!("Existing socket is not present");
        // The socket file doesn't exist, so the socket can't exist.
        return false;
    }

    debug!("Attempting to Connect to Existing Socket");
    // The socket exists, let's see if we can connect to it
    match UnixStream::connect(&socket_path) {
        Ok(mut stream) => {
            debug!("Connected to Existing Socket at {socket_path:?}, Sending Trigger");
            let _ = stream.write_all(b"TRIGGER");
            return true;
        }
        Err(e) => {
            debug!("Failed to Connect to Socket: {e}");
            debug!("Removing Stale Socket File");
            let _ = fs::remove_file(socket_path);
        }
    }
    false
}

fn get_socket_file_path() -> PathBuf {
    let mut path = runtime_dir().unwrap_or_else(env::temp_dir);
    path.push(format!("{}.sock", APP_NAME));

    path
}

pub fn display_error(message: String) {
    use std::process::Command;
    // We have two choices here, kdialog, or zenity. We'll try both.
    if let Err(e) = Command::new("kdialog")
        .arg("--title")
        .arg("Pipeweaver UI")
        .arg("--error")
        .arg(message.clone())
        .output()
    {
        println!("Error Running kdialog: {e}, falling back to zenity..");
        let _ = Command::new("zenity")
            .arg("--title")
            .arg("Pipeweaver UI")
            .arg("--error")
            .arg("--text")
            .arg(message)
            .output();
    }
}
