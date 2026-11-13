//! The default Roblox launcher: the program Windows opens Roblox's links with (`roblox-player:`
//! from the website's Play button, `roblox:` from the app and share links). Roblox, a
//! bootstrapper (RoALT then launches through it, `client::find`) or RoALT itself, which starts
//! the client at once for a `roblox-player:` link (`open`) and fills in Play for a `roblox:` one.
//!
//! The registry (HKCU, no admin) is the truth. `HOLD` only says RoALT should keep the links:
//! Roblox's installer takes them back on every update (`reclaim`). The command RoALT replaced
//! stays on each link's key as `RoALTBefore`, which the uninstaller puts back.

use std::path::Path;

use tauri::{AppHandle, Manager};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, REG_SZ, RegSetKeyValueW};
use windows::core::HSTRING;

use super::{client, client_settings, update};
use crate::App;
use crate::error::{AppError, Result};
use crate::model::{LauncherKind, RobloxLauncher};
use crate::vault::Secret;

const SCHEMES: [&str; 2] = ["roblox-player", "roblox"];
const BEFORE: &str = "RoALTBefore";
/// Kept beside the settings (`setting_value`): "1" while RoALT should hold the links.
const HOLD: &str = "roblox_links";

fn kind_of(exe: &Path) -> LauncherKind {
    if client::is_roalt(exe) {
        LauncherKind::Roalt
    } else if client::is_roblox(exe) {
        LauncherKind::Roblox
    } else {
        LauncherKind::Bootstrapper
    }
}

/// Roblox's own client (if installed), each bootstrapper installed, the program holding the
/// links if it is none of those, then RoALT.
pub fn list() -> Vec<RobloxLauncher> {
    let handler = client::handler();
    let mut found: Vec<(LauncherKind, std::path::PathBuf)> = Vec::new();
    found.extend(client::roblox_own().map(|p| (LauncherKind::Roblox, p)));
    found.extend(client::bootstrappers().into_iter().map(|p| (LauncherKind::Bootstrapper, p)));
    if let Some(h) = handler.as_ref().filter(|h| kind_of(h) == LauncherKind::Bootstrapper && !found.iter().any(|(_, p)| client::same_path(p, h))) {
        found.push((LauncherKind::Bootstrapper, h.clone()));
    }
    found.extend(std::env::current_exe().ok().map(|p| (LauncherKind::Roalt, p)));
    found
        .into_iter()
        .map(|(kind, path)| RobloxLauncher {
            kind,
            name: match kind {
                LauncherKind::Roblox => "Roblox".into(),
                LauncherKind::Roalt => "RoALT".into(),
                LauncherKind::Bootstrapper => path.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned()),
            },
            // Roblox's own may be registered from another version folder; another RoALT install isn't this one.
            current: handler.as_ref().is_some_and(|h| kind_of(h) == kind && (kind == LauncherKind::Roblox || client::same_path(h, &path))),
            path: path.display().to_string(),
        })
        .collect()
}

/// Makes the launcher at `path`, one `list` offers (nothing else is ever registered), open
/// Roblox's links; RoALT then holds them or lets go. Returns its kind.
pub fn set(s: &App, path: &str) -> Result<LauncherKind> {
    let pick = list()
        .into_iter()
        .find(|l| client::same_path(Path::new(&l.path), Path::new(path)))
        .ok_or_else(|| AppError::Invalid(tr!("That launcher isn't installed anymore. Pick another one.")))?;
    // Already so: an unknown bootstrapper keeps its own command.
    if !pick.current {
        write(pick.kind, Path::new(&pick.path))?;
    }
    s.db.set_setting_value(HOLD, (pick.kind == LauncherKind::Roalt).then_some("1"))?;
    log::info!("default Roblox launcher: {} ({})", pick.name, pick.path);
    Ok(pick.kind)
}

/// The `shell\open\command` for a launcher (pure): Roblox's and the Bloxstrap family's own forms.
fn command(kind: LauncherKind, exe: &Path) -> String {
    let exe = exe.display();
    match kind {
        LauncherKind::Roblox => format!("\"{exe}\" %1"),
        LauncherKind::Bootstrapper => format!("\"{exe}\" -player \"%1\""),
        LauncherKind::Roalt => format!("\"{exe}\" \"%1\""),
    }
}

/// Registers `exe` for both link schemes in HKCU. Taking them for RoALT keeps the command it
/// replaces, unless that was RoALT's.
fn write(kind: LauncherKind, exe: &Path) -> Result<()> {
    let command = command(kind, exe);
    for scheme in SCHEMES {
        let key = format!(r"Software\Classes\{scheme}");
        let open = format!(r"{key}\shell\open\command");
        if kind == LauncherKind::Roalt
            && let Some(before) = client::reg_string(HKEY_CURRENT_USER, &open, None)
            && !client::split_command(&before).is_some_and(|(e, _)| client::is_roalt(&e))
        {
            reg_set(&key, Some(BEFORE), &before)?;
        }
        reg_set(&key, None, "URL: Roblox Protocol")?;
        reg_set(&key, Some("URL Protocol"), "")?;
        reg_set(&open, None, &command)?;
    }
    Ok(())
}

/// A `REG_SZ` in HKCU, the key made if missing; `None` names the key's default value.
fn reg_set(key: &str, name: Option<&str>, value: &str) -> Result<()> {
    let (key, name) = (HSTRING::from(key), name.map(HSTRING::from));
    let data: Vec<u16> = value.encode_utf16().chain([0]).collect();
    // SAFETY: `key`, `name` and `data` are live NUL-terminated wide strings; `data` is
    // `data.len() * 2` bytes.
    let r = unsafe {
        RegSetKeyValueW(HKEY_CURRENT_USER, &key, name.as_ref().map_or(windows::core::PCWSTR::null(), |n| windows::core::PCWSTR(n.as_ptr())), REG_SZ.0, Some(data.as_ptr().cast()), (data.len() * 2) as u32)
    };
    r.ok().map_err(|e| AppError::Internal(tr!("Could not change which program opens Roblox links: {why}", why = e)))
}

#[derive(Debug, PartialEq, Eq)]
enum Reclaim {
    Nothing,
    Take,
    LetGo,
}

/// What RoALT does about the links (pure): while it should hold them and doesn't, it takes them
/// back from Roblox (its installer rewrites them) or nobody; a bootstrapper taking them means
/// the user moved on.
fn reclaim_choice(hold: bool, holder: Option<LauncherKind>) -> Reclaim {
    match (hold, holder) {
        (false, _) | (true, Some(LauncherKind::Roalt)) => Reclaim::Nothing,
        (true, Some(LauncherKind::Bootstrapper)) => Reclaim::LetGo,
        (true, Some(LauncherKind::Roblox) | None) => Reclaim::Take,
    }
}

/// At start, every 30 minutes and after an install (`update::publish`).
pub fn reclaim(s: &App) {
    let hold = s.db.setting_value(HOLD).ok().flatten().is_some();
    let holder = client::handler();
    match reclaim_choice(hold, holder.as_deref().map(kind_of)) {
        Reclaim::Nothing => {}
        Reclaim::Take => match std::env::current_exe().map_err(AppError::from).and_then(|me| write(LauncherKind::Roalt, &me)) {
            Ok(()) => log::info!("Roblox links taken back from {holder:?}"),
            Err(e) => log::warn!("could not take Roblox links back: {e}"),
        },
        Reclaim::LetGo => {
            log::info!("Roblox links: {holder:?} holds them now; RoALT no longer takes them back");
            if let Err(e) = s.db.set_setting_value(HOLD, None) {
                log::warn!("could not save that RoALT lets go of Roblox links: {e}");
            }
        }
    }
}

const LAUNCH_PREFIX: &str = "roblox-player:1+launchmode:";

/// The website's launch link as Roblox gets it (pure): `roblox-player:1+launchmode:…`, at most
/// 8 KB of printable ASCII with no spaces, quotes or shell characters, as it goes on the
/// client's command line raw.
fn launch_link(arg: &str) -> Option<&str> {
    let ok = arg.len() <= 8192
        && arg.get(..LAUNCH_PREFIX.len()).is_some_and(|p| p.eq_ignore_ascii_case(LAUNCH_PREFIX))
        && arg.bytes().all(|b| b.is_ascii_graphic() && !b"\"'^&|<>".contains(&b));
    ok.then_some(arg)
}

/// A `roblox-player:` link (the website's Play button, while RoALT opens Roblox's links): starts
/// the client at once, as Roblox would, as the browser's account. RoALT doesn't know that
/// account, so the client isn't followed. Modpacks apply. The link is never logged.
pub fn open(app: &AppHandle, arg: &str) {
    let Some(link) = launch_link(arg).map(Secret::new) else {
        log::warn!("ignored a roblox-player: link that isn't a game launch");
        return;
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = start(&app, &link).await {
            crate::commands::notice(&app, true, e.message());
        }
    });
}

async fn start(app: &AppHandle, link: &Secret) -> Result<()> {
    let s = app.state::<App>();
    if s.update.installing() {
        return Err(AppError::Invalid(tr!("Roblox is being installed. Launch again once it is done.")));
    }
    // Not while a RoALT launch has its own values in Roblox's settings file, and none until this
    // client has read the file: a RoALT launch right after would hand it that account's FPS cap.
    let _held = client_settings::LOCK.lock().await;
    let (exe, _, self_updates) = update::launch_client(&s).await?;
    let (exe, skipped) = tokio::task::block_in_place(|| crate::mods::runtime::prepare(&s, exe, self_updates));
    if let Some(why) = skipped {
        log::info!("Modpacks not applied to a link from the browser: {why}");
    }
    let since = std::time::SystemTime::now();
    let (pid, pid_start) = client::start(&exe, link, &[])?;
    log::info!("started a roblox-player: link from the browser (pid {pid}, bootstrapper: {})", exe.bootstrapper);
    if let Some(id) = tracker_id(link.expose()) {
        // A bootstrapper's own process exits by design: only the log tells then.
        client_settings::wait_until_read(id, since, (!exe.bootstrapper).then_some((pid, pid_start))).await;
    }
    Ok(())
}

/// The `browsertrackerid:` of a launch link (pure): the client's log names it.
fn tracker_id(link: &str) -> Option<i64> {
    link.split('+').find_map(|p| p.strip_prefix("browsertrackerid:"))?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_for_each_launcher() {
        let exe = Path::new(r"C:\X\a.exe");
        assert_eq!(command(LauncherKind::Roblox, exe), r#""C:\X\a.exe" %1"#);
        assert_eq!(command(LauncherKind::Bootstrapper, exe), r#""C:\X\a.exe" -player "%1""#);
        assert_eq!(command(LauncherKind::Roalt, exe), r#""C:\X\a.exe" "%1""#);
        assert_eq!(kind_of(Path::new(r"C:\P\RoALT\RoALT.exe")), LauncherKind::Roalt);
        assert_eq!(kind_of(Path::new(r"C:\R\Versions\v\RobloxPlayerBeta.exe")), LauncherKind::Roblox);
        assert_eq!(kind_of(Path::new(r"C:\B\Bloxstrap.exe")), LauncherKind::Bootstrapper);
    }

    #[test]
    fn takes_the_links_back_only_from_roblox() {
        use LauncherKind::*;
        assert_eq!(reclaim_choice(false, Some(Roblox)), Reclaim::Nothing);
        assert_eq!(reclaim_choice(true, Some(Roalt)), Reclaim::Nothing);
        assert_eq!(reclaim_choice(true, Some(Roblox)), Reclaim::Take);
        assert_eq!(reclaim_choice(true, None), Reclaim::Take);
        assert_eq!(reclaim_choice(true, Some(Bootstrapper)), Reclaim::LetGo);
    }

    #[test]
    fn only_a_launch_link_goes_to_the_client() {
        let good = "roblox-player:1+launchmode:play+gameinfo:abc_DEF-123+launchtime:1792839000000+placelauncherurl:https%3A%2F%2Fwww.roblox.com%2FGame%2FPlaceLauncher.ashx%3Frequest%3DRequestGame%26placeId%3D1818+browsertrackerid:1+robloxLocale:en_us+gameLocale:en_us+LaunchExp:InApp";
        assert_eq!(launch_link(good), Some(good));
        assert_eq!(tracker_id(good), Some(1));
        assert_eq!(tracker_id("roblox-player:1+launchmode:play+browsertrackerid:x"), None);
        assert_eq!(tracker_id("roblox-player:1+launchmode:play"), None);
        for bad in [
            "roblox-player:1+launchmode:play+gameinfo:x --other",
            "roblox-player:1+launchmode:play\"+-flag",
            "roblox-player:1+launchmode:play&calc",
            "roblox-player:1+launchmode:play\n",
            "roblox-player:1+gameinfo:x",
            "roblox://experiences/start?placeId=1818",
            "",
        ] {
            assert_eq!(launch_link(bad), None, "{bad:?}");
        }
        assert_eq!(launch_link(&format!("{LAUNCH_PREFIX}{}", "a".repeat(8192))), None, "too long");
    }

    /// The registry write on a throwaway key: made when missing, read back, removed.
    #[test]
    fn writes_a_value_in_a_new_key() {
        use windows::Win32::System::Registry::{RegDeleteKeyW, RegDeleteTreeW};
        let root = format!(r"Software\RoALT-test-{}", uuid::Uuid::new_v4());
        let key = format!(r"{root}\shell\open\command");
        reg_set(&key, None, r#""C:\X\a.exe" "%1""#).unwrap();
        reg_set(&key, Some(BEFORE), "old").unwrap();
        let (default, before) = (client::reg_string(HKEY_CURRENT_USER, &key, None), client::reg_string(HKEY_CURRENT_USER, &key, Some(BEFORE)));
        let root = HSTRING::from(root.as_str());
        // SAFETY: a live NUL-terminated string naming only this test's key (its tree, then itself).
        let _ = unsafe { (RegDeleteTreeW(HKEY_CURRENT_USER, &root), RegDeleteKeyW(HKEY_CURRENT_USER, &root)) };
        assert_eq!(default.as_deref(), Some(r#""C:\X\a.exe" "%1""#));
        assert_eq!(before.as_deref(), Some("old"));
    }
}
