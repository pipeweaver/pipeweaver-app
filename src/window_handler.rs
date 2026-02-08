use qmetaobject::prelude::*;
use std::sync::mpsc;

pub enum WindowMessage {
    Trigger,
    Close,
}

#[derive(QObject)]
pub struct WindowHandler {
    rx: mpsc::Receiver<WindowMessage>,
    base: qt_base_class!(trait QObject),

    // Called to focus the QT Window
    trigger: qt_signal!(),
    on_trigger: qt_method!(
        fn on_trigger(&self) {
            self.trigger();
        }
    ),

    // Called to close the QT Window
    close: qt_signal!(),
    on_close: qt_method!(
        fn on_close(&self) {
            self.close();
        }
    ),

    // Called from QT to probe the message queue
    check_notifications: qt_method!(
        fn check_notifications(&self) {
            while let Ok(msg) = self.rx.try_recv() {
                match msg {
                    WindowMessage::Trigger => {
                        self.on_trigger();
                    }
                    WindowMessage::Close => {
                        // Handle close request from IPC
                        self.on_close();
                    }
                }
            }
        }
    ),
}

impl WindowHandler {
    pub fn new(rx: mpsc::Receiver<WindowMessage>) -> Self {
        Self {
            rx,
            base: Default::default(),

            trigger: Default::default(),
            on_trigger: Default::default(),

            close: Default::default(),
            on_close: Default::default(),

            check_notifications: Default::default(),
        }
    }
}
