use super::super::{Event, Events};
use std::sync::Mutex;

static SINK: Mutex<Option<Events>> = Mutex::new(None);
unsafe extern "C" {
    fn sb_install_system_events(callback: extern "C" fn(u32)) -> bool;
    fn sb_remove_system_events();
}
extern "C" fn receive(event: u32) {
    let sink = SINK.lock().unwrap().clone();
    if let Some(sink) = sink {
        match event {
            1 => sink.emit(Event::Sleep),
            2 => sink.emit(Event::Wake),
            3 => sink.emit(Event::Displays),
            4 => sink.emit(Event::ScreenWake),
            _ => {}
        }
    }
}
pub struct Registration;
impl Registration {
    pub fn install(events: Events) -> anyhow::Result<Self> {
        *SINK.lock().unwrap() = Some(events);
        if !unsafe { sb_install_system_events(receive) } {
            SINK.lock().unwrap().take();
            anyhow::bail!("macOS 原生休眠/显示通知注册失败");
        }
        Ok(Self)
    }
}
impl Drop for Registration {
    fn drop(&mut self) {
        SINK.lock().unwrap().take();
        unsafe {
            sb_remove_system_events();
        }
    }
}
