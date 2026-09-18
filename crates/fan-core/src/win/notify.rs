//! Endpoint notifications, forwarded to the supervisor thread.

use std::sync::mpsc::Sender;

use parking_lot::Mutex;
use windows::core::{implement, IUnknownImpl, PCWSTR};
use windows::Win32::Media::Audio::{
    eRender, EDataFlow, ERole, IMMNotificationClient, IMMNotificationClient_Impl, DEVICE_STATE,
    DEVICE_STATE_ACTIVE,
};
use windows::Win32::UI::Shell::PropertiesSystem::PROPERTYKEY;

use crate::win::Cmd;

#[implement(IMMNotificationClient)]
pub(crate) struct Notifier {
    tx: Mutex<Sender<Cmd>>,
}

impl Notifier {
    pub(crate) fn new(tx: Sender<Cmd>) -> Self {
        Self { tx: Mutex::new(tx) }
    }

    fn send(&self, cmd: Cmd) {
        let _ = self.tx.lock().send(cmd);
    }
}

unsafe fn text(id: &PCWSTR) -> String {
    if id.is_null() {
        String::new()
    } else {
        id.to_string().unwrap_or_default()
    }
}

impl IMMNotificationClient_Impl for Notifier_Impl {
    fn OnDeviceStateChanged(
        &self,
        pwstrdeviceid: &PCWSTR,
        dwnewstate: DEVICE_STATE,
    ) -> windows::core::Result<()> {
        let id = unsafe { text(pwstrdeviceid) };
        if dwnewstate == DEVICE_STATE_ACTIVE {
            self.get_impl().send(Cmd::DeviceAvailable(id));
        } else {
            self.get_impl().send(Cmd::DeviceLost(id));
        }
        Ok(())
    }

    fn OnDeviceAdded(&self, pwstrdeviceid: &PCWSTR) -> windows::core::Result<()> {
        self.get_impl()
            .send(Cmd::DeviceAvailable(unsafe { text(pwstrdeviceid) }));
        Ok(())
    }

    fn OnDeviceRemoved(&self, pwstrdeviceid: &PCWSTR) -> windows::core::Result<()> {
        self.get_impl()
            .send(Cmd::DeviceLost(unsafe { text(pwstrdeviceid) }));
        Ok(())
    }

    fn OnDefaultDeviceChanged(
        &self,
        flow: EDataFlow,
        _role: ERole,
        pwstrdefaultdeviceid: &PCWSTR,
    ) -> windows::core::Result<()> {
        if flow == eRender {
            self.get_impl().send(Cmd::DefaultRenderChanged(unsafe {
                text(pwstrdefaultdeviceid)
            }));
        }
        Ok(())
    }

    fn OnPropertyValueChanged(
        &self,
        _pwstrdeviceid: &PCWSTR,
        _key: &PROPERTYKEY,
    ) -> windows::core::Result<()> {
        Ok(())
    }
}
