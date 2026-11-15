use std::ffi::OsString;
use std::fs::Metadata;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::{Path, PathBuf};

use tokio::net::UnixStream;

use super::client::TIMEOUT;
use crate::runtime::Error;

/// Where the `rex` CLI looks for the local server, from `var` (collied's own environment):
/// `REX_RUNTIME_DIR`, else `XDG_RUNTIME_DIR/rex`, else the platform's folder. Never
/// `REX_SERVER`, which can name another machine.
pub fn socket_path(var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let set = |name: &str| var(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    if let Some(dir) = set("REX_RUNTIME_DIR") {
        return Some(dir.join("server.sock"));
    }
    if let Some(dir) = set("XDG_RUNTIME_DIR") {
        return Some(dir.join("rex/server.sock"));
    }
    platform(&set)
}

#[cfg(target_os = "macos")]
fn platform(set: &dyn Fn(&str) -> Option<PathBuf>) -> Option<PathBuf> {
    Some(set("HOME")?.join("Library/Application Support/rex/server.sock"))
}

#[cfg(not(target_os = "macos"))]
fn platform(_: &dyn Fn(&str) -> Option<PathBuf>) -> Option<PathBuf> {
    let uid = rustix::process::getuid().as_raw();
    Some(PathBuf::from(format!("/run/user/{uid}/rex/server.sock")))
}

/// Why a file keeps out none but its owner, or None when it does.
fn private(meta: &Metadata, uid: u32) -> Option<String> {
    let mode = meta.mode() & 0o777;
    if meta.uid() != uid {
        Some(format!("owned by uid {}", meta.uid()))
    } else if mode & 0o077 != 0 {
        Some(format!("mode {mode:04o}, open to other users"))
    } else {
        None
    }
}

/// A personal server only: its folder a real folder and its socket a socket, both owned by
/// the user with no group or other bits. That keeps out Rex's shared server, whose group
/// members all type into every session as the `rex` account, and a personal socket opened
/// to a group. A missing folder or socket is an `Io` error: no Rex runs.
fn check(socket: &Path, uid: u32) -> Result<(), Error> {
    let folder = socket.parent().unwrap_or(Path::new("/"));
    let meta = std::fs::symlink_metadata(folder)?;
    let problem = if meta.is_dir() {
        private(&meta, uid)
    } else {
        Some("not a folder".to_owned())
    };
    if let Some(why) = problem {
        return Err(Error::Unsupported(format!(
            "has its socket in {}, {why}; collied follows only a personal Rex server",
            folder.display()
        )));
    }
    let meta = std::fs::symlink_metadata(socket)?;
    let problem = if meta.file_type().is_socket() {
        private(&meta, uid)
    } else {
        Some("not a socket".to_owned())
    };
    if let Some(why) = problem {
        return Err(Error::Unsupported(format!(
            "has its socket {}, {why}; collied follows only a personal Rex server",
            socket.display()
        )));
    }
    Ok(())
}

/// The folder and socket checks, then the server's own uid from the socket.
pub(super) async fn connect(socket: &Path) -> Result<UnixStream, Error> {
    let uid = rustix::process::geteuid().as_raw();
    check(socket, uid)?;
    let stream = tokio::time::timeout(TIMEOUT, UnixStream::connect(socket))
        .await
        .map_err(|_| Error::Timeout)??;
    if stream.peer_cred()?.uid() != uid {
        return Err(Error::Unsupported(
            "is served by another user; collied follows only a personal Rex server".to_owned(),
        ));
    }
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn env(vars: &[(&'static str, &'static str)]) -> impl Fn(&str) -> Option<OsString> {
        let vars = vars.to_vec();
        move |k| {
            vars.iter()
                .find(|(name, _)| *name == k)
                .map(|(_, v)| OsString::from(v))
        }
    }

    #[test]
    fn the_socket_is_found_where_the_rex_cli_looks() {
        let path = |vars| socket_path(env(vars)).unwrap();
        assert_eq!(
            path(&[
                ("REX_RUNTIME_DIR", "/r"),
                ("XDG_RUNTIME_DIR", "/x"),
                ("HOME", "/h")
            ]),
            Path::new("/r/server.sock")
        );
        assert_eq!(
            path(&[
                ("REX_RUNTIME_DIR", ""),
                ("XDG_RUNTIME_DIR", "/x"),
                ("HOME", "/h")
            ]),
            Path::new("/x/rex/server.sock")
        );
        // REX_SERVER names a server anywhere, another machine's included; it is never followed.
        let fallback = path(&[("REX_SERVER", "tcp://elsewhere:7297"), ("HOME", "/h")]);
        assert!(fallback.ends_with("rex/server.sock"), "{fallback:?}");
        #[cfg(target_os = "macos")]
        assert_eq!(
            fallback,
            Path::new("/h/Library/Application Support/rex/server.sock")
        );
    }

    fn mode(path: &Path, mode: u32) {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    #[test]
    fn only_a_personal_servers_folder_and_socket_are_followed() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("rex");
        std::fs::create_dir(&folder).unwrap();
        let socket = folder.join("server.sock");
        let uid = rustix::process::geteuid().as_raw();
        let refused = |socket: &Path, uid| match check(socket, uid) {
            Err(Error::Unsupported(why)) => why,
            other => panic!("{other:?}"),
        };

        mode(&folder, 0o700);
        assert!(
            matches!(check(&socket, uid), Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound)
        );
        let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        mode(&socket, 0o600);
        check(&socket, uid).unwrap();

        mode(&socket, 0o660);
        assert!(refused(&socket, uid).contains("0660"));
        mode(&socket, 0o600);
        mode(&folder, 0o750);
        assert!(refused(&socket, uid).contains("0750"));
        mode(&folder, 0o700);
        assert!(refused(&socket, uid + 1).contains(&format!("owned by uid {uid}")));

        let file = folder.join("file.sock");
        std::fs::write(&file, "").unwrap();
        mode(&file, 0o600);
        assert!(refused(&file, uid).contains("not a socket"));
        let link = root.path().join("link");
        std::os::unix::fs::symlink(&folder, &link).unwrap();
        assert!(refused(&link.join("server.sock"), uid).contains("not a folder"));
    }
}
