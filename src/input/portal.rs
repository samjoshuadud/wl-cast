use std::fmt::Display;

use ashpd::desktop::{
    PersistMode, Session,
    remote_desktop::{
        DeviceType, KeyState, NotifyPointerAxisOptions, RemoteDesktop, SelectDevicesOptions,
    },
    screencast::{Screencast, SelectSourcesOptions, SourceType},
};
use futures_lite::future::block_on;

use super::{MouseButton, PressState};
use crate::Error;

const XKB_KEYCODE_OFFSET: i32 = 8;

pub(super) struct PortalInput {
    proxy: RemoteDesktop,
    session: Session<RemoteDesktop>,
    stream: u32,
    width: f64,
    height: f64,
}

impl PortalInput {
    pub(super) fn new() -> Result<Self, Error> {
        let (proxy, session, stream, width, height) = block_on(start())?;
        Ok(Self {
            proxy,
            session,
            stream,
            width,
            height,
        })
    }

    pub(super) fn move_to(&self, x: f64, y: f64) -> Result<(), Error> {
        let (x, y) = absolute_position(x, y, self.width, self.height);
        block_on(self.proxy.notify_pointer_motion_absolute(
            &self.session,
            self.stream,
            x,
            y,
            Default::default(),
        ))
        .map_err(portal_error)
    }

    pub(super) fn button(&self, button: MouseButton, press: PressState) -> Result<(), Error> {
        let code = i32::try_from(button.linux_code())
            .map_err(|_| Error::Portal("mouse button does not fit".into()))?;
        block_on(self.proxy.notify_pointer_button(
            &self.session,
            code,
            key_state(press),
            Default::default(),
        ))
        .map_err(portal_error)
    }

    pub(super) fn scroll(&self, dx: f64, dy: f64) -> Result<(), Error> {
        block_on(self.proxy.notify_pointer_axis(
            &self.session,
            dx,
            dy,
            NotifyPointerAxisOptions::default().set_finish(true),
        ))
        .map_err(portal_error)
    }

    pub(super) fn key(&self, code: u32, press: PressState) -> Result<(), Error> {
        block_on(self.proxy.notify_keyboard_keycode(
            &self.session,
            portal_keycode(code)?,
            key_state(press),
            Default::default(),
        ))
        .map_err(portal_error)
    }
}

impl Drop for PortalInput {
    fn drop(&mut self) {
        let _ = block_on(self.session.close());
    }
}

impl std::fmt::Debug for PortalInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PortalInput")
            .field("stream", &self.stream)
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

async fn start() -> Result<(RemoteDesktop, Session<RemoteDesktop>, u32, f64, f64), Error> {
    let remote = RemoteDesktop::new().await.map_err(portal_error)?;
    let session = remote
        .create_session(Default::default())
        .await
        .map_err(portal_error)?;
    match prepare(&remote, &session).await {
        Ok((stream, width, height)) => Ok((remote, session, stream, width, height)),
        Err(error) => {
            let _ = session.close().await;
            Err(error)
        }
    }
}

async fn prepare(
    remote: &RemoteDesktop,
    session: &Session<RemoteDesktop>,
) -> Result<(u32, f64, f64), Error> {
    remote
        .select_devices(
            session,
            SelectDevicesOptions::default()
                .set_devices(DeviceType::Keyboard | DeviceType::Pointer)
                .set_persist_mode(PersistMode::DoNot),
        )
        .await
        .map_err(portal_error)?
        .response()
        .map_err(portal_error)?;
    Screencast::new()
        .await
        .map_err(portal_error)?
        .select_sources(
            session,
            SelectSourcesOptions::default()
                .set_sources(ashpd::enumflags2::BitFlags::from_flag(SourceType::Monitor))
                .set_multiple(false)
                .set_persist_mode(PersistMode::DoNot),
        )
        .await
        .map_err(portal_error)?
        .response()
        .map_err(portal_error)?;
    let started = remote
        .start(session, None, Default::default())
        .await
        .map_err(portal_error)?
        .response()
        .map_err(portal_error)?;
    let stream = started
        .streams()
        .first()
        .ok_or_else(|| Error::Portal("the portal shared no monitor".into()))?;
    let (width, height) = stream
        .size()
        .ok_or_else(|| Error::Portal("the portal shared no monitor size".into()))?;
    if width <= 0 || height <= 0 {
        return Err(Error::Portal("the portal shared no monitor size".into()));
    }
    Ok((stream.pipe_wire_node_id(), width as f64, height as f64))
}

fn absolute_position(x: f64, y: f64, width: f64, height: f64) -> (f64, f64) {
    (x.clamp(0.0, 1.0) * width, y.clamp(0.0, 1.0) * height)
}

fn portal_keycode(code: u32) -> Result<i32, Error> {
    i32::try_from(code)
        .ok()
        .and_then(|code| code.checked_add(XKB_KEYCODE_OFFSET))
        .ok_or_else(|| Error::Portal("key code does not fit".into()))
}

fn key_state(press: PressState) -> KeyState {
    match press {
        PressState::Pressed => KeyState::Pressed,
        PressState::Released => KeyState::Released,
    }
}

fn portal_error(error: impl Display) -> Error {
    Error::Portal(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fractions_of_the_screen_become_pixels() {
        assert_eq!(absolute_position(0.5, 0.25, 200.0, 100.0), (100.0, 25.0));
        assert_eq!(absolute_position(-1.0, 2.0, 200.0, 100.0), (0.0, 100.0));
    }

    #[test]
    fn linux_key_codes_become_xkb_keycodes() {
        assert_eq!(portal_keycode(30).unwrap(), 38);
        assert!(portal_keycode(u32::MAX).is_err());
    }

    #[test]
    fn press_states_match_the_portal() {
        assert_eq!(key_state(PressState::Pressed), KeyState::Pressed);
        assert_eq!(key_state(PressState::Released), KeyState::Released);
    }
}
