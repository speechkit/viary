//! Typing on Wayland through GNOME's RemoteDesktop portal. GNOME asks the
//! first time; the restore token it hands back skips the question after
//! that. A session is opened for each paste, so the top bar's sharing icon
//! shows only while Viary types.

use ashpd::{
    desktop::{
        PersistMode,
        remote_desktop::{DeviceType, KeyState, RemoteDesktop, SelectDevicesOptions},
    },
    enumflags2::BitFlags,
};

use super::{change_prefs, prefs};

/// Presses `keysyms` in order and releases them in reverse.
pub fn press(keysyms: &[u32]) -> Result<(), String> {
    tauri::async_runtime::block_on(session(keysyms)).map_err(|e| e.to_string())
}

/// Asks GNOME for keyboard access now, with nothing to type.
pub fn allow() -> Result<(), String> {
    press(&[])
}

async fn session(keysyms: &[u32]) -> ashpd::Result<()> {
    let proxy = RemoteDesktop::new().await?;
    let session = proxy.create_session(Default::default()).await?;
    let token = prefs().restore_token;
    proxy
        .select_devices(
            &session,
            SelectDevicesOptions::default()
                .set_devices(BitFlags::from(DeviceType::Keyboard))
                .set_persist_mode(PersistMode::ExplicitlyRevoked)
                .set_restore_token(token.as_deref()),
        )
        .await?
        .response()?;
    let started = proxy.start(&session, None, Default::default()).await?.response()?;
    // Each start hands out a new token; the old one no longer works.
    if let Some(token) = started.restore_token() {
        let token = token.to_owned();
        change_prefs(|p| p.restore_token = Some(token));
    }
    let keysyms: Vec<i32> = keysyms.iter().filter_map(|&k| i32::try_from(k).ok()).collect();
    for &keysym in &keysyms {
        proxy
            .notify_keyboard_keysym(&session, keysym, KeyState::Pressed, Default::default())
            .await?;
    }
    for &keysym in keysyms.iter().rev() {
        proxy
            .notify_keyboard_keysym(&session, keysym, KeyState::Released, Default::default())
            .await?;
    }
    session.close().await?;
    Ok(())
}
