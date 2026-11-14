use eframe::egui;
use interprocess::local_socket::{prelude::*, GenericNamespaced, ListenerOptions};
use std::ffi::OsString;
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, Read, Write};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread;
use std::time::{Duration, Instant};
use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, IsIconic, MessageBoxW, SetForegroundWindow, ShowWindow,
    MB_ICONERROR, MB_OK, SW_RESTORE,
};

const TIMEOUT: Duration = Duration::from_secs(10);
const MAX_MESSAGE: usize = 512 * 1024;
const PAUSE: Duration = Duration::from_millis(10);

pub(crate) fn report_error(error: &str) {
    let text: Vec<u16> = OsString::from(error).encode_wide().chain(Some(0)).collect();
    let title: Vec<u16> = OsString::from("RustEd").encode_wide().chain(Some(0)).collect();
    unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), MB_OK | MB_ICONERROR); }
}

pub(crate) struct Launch {
    pub(crate) file: Option<PathBuf>,
    pub(crate) tail: bool,
    pub(crate) error: Option<String>,
    pub(crate) focus_error: Option<String>,
}

pub(crate) enum Instance {
    Primary { lock: File, listener: LocalSocketListener },
    Secondary { name: String },
}

impl Instance {
    pub(crate) fn acquire() -> io::Result<Self> {
        let config = crate::settings::recent_file()
            .ok_or_else(|| io::Error::other("Cannot determine the application settings directory"))?;
        let directory = config.parent().ok_or_else(|| io::Error::other("Invalid settings directory"))?;
        let mut session = 0;
        // The pipe namespace is shared across sessions, while activation is session-specific.
        if unsafe { ProcessIdToSessionId(std::process::id(), &mut session) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Self::acquire_at(directory, session)
    }

    fn acquire_at(directory: &Path, session: u32) -> io::Result<Self> {
        std::fs::create_dir_all(directory)?;
        let directory = std::fs::canonicalize(directory)?;
        let mut hash = 0xcbf29ce484222325_u64;
        for unit in directory.as_os_str().encode_wide() {
            for byte in unit.to_le_bytes() {
                hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
            }
        }
        let name = format!("rusted-v1-{session}-{hash:016x}");
        let lock = OpenOptions::new().read(true).write(true).create(true).truncate(false)
            .open(directory.join(format!("instance-{session}.lock")))?;
        match lock.try_lock() {
            Ok(()) => {
                let socket_name = name.as_str().to_ns_name::<GenericNamespaced>()?;
                let listener = ListenerOptions::new().name(socket_name).create_sync()?;
                Ok(Self::Primary { lock, listener })
            }
            Err(TryLockError::WouldBlock) => Ok(Self::Secondary { name }),
            Err(TryLockError::Error(error)) => Err(error),
        }
    }
}

fn transfer(
    stream: &mut LocalSocketStream,
    data: &mut [u8],
    writing: bool,
    deadline: Instant,
) -> io::Result<()> {
    let mut offset = 0;
    while offset < data.len() {
        let result = if writing {
            stream.write(&data[offset..])
        } else {
            stream.read(&mut data[offset..])
        };
        match result {
            // Windows nonblocking pipes can report zero when no data is available.
            Ok(0) if Instant::now() < deadline => thread::sleep(PAUSE),
            Ok(0) => return Err(io::Error::new(io::ErrorKind::TimedOut, "Instance connection timed out")),
            Ok(n) => offset += n,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline => {
                thread::sleep(PAUSE);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "Instance connection timed out"));
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

pub(crate) fn forward(name: &str, args: impl IntoIterator<Item = OsString>) -> io::Result<()> {
    let deadline = Instant::now() + TIMEOUT;
    let name = name.to_owned();
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let result = loop {
            let socket_name = match name.as_str().to_ns_name::<GenericNamespaced>() {
                Ok(name) => name,
                Err(error) => break Err(error),
            };
            match LocalSocketStream::connect(socket_name) {
                Ok(stream) => break Ok(stream),
                Err(error) if Instant::now() < deadline &&
                    matches!(error.kind(), io::ErrorKind::NotFound | io::ErrorKind::WouldBlock) => thread::sleep(PAUSE),
                Err(error) => break Err(error),
            }
        };
        let _ = sender.send(result);
    });
    let mut stream = receiver.recv_timeout(TIMEOUT)
        .map_err(|error| io::Error::other(format!("Cannot connect to running instance: {error}")))??;
    stream.set_nonblocking(true)?;
    let mut pid = [0; 4];
    transfer(&mut stream, &mut pid, false, deadline)?;
    let pid = u32::from_le_bytes(pid);
    let focus_error = if unsafe { AllowSetForegroundWindow(pid) } == 0 {
        Some(format!("Could not grant foreground permission: {}", io::Error::last_os_error()))
    } else {
        None
    };
    let (file, tail, error) = match crate::parse_args(args) {
        Ok((Some(file), tail)) => match std::path::absolute(file) {
            Ok(file) => (Some(file.as_os_str().encode_wide().collect::<Vec<_>>()), tail, None),
            Err(error) => (None, false, Some(format!("Cannot resolve file path: {error}"))),
        },
        Ok((None, tail)) => (None, tail, None),
        Err(error) => (None, false, Some(error.to_owned())),
    };
    let mut payload = serde_json::to_vec(&(file, tail, error, focus_error))?;
    if payload.len() > MAX_MESSAGE {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Instance request is too large"));
    }
    let mut size = (payload.len() as u32).to_le_bytes();
    transfer(&mut stream, &mut size, true, deadline)?;
    transfer(&mut stream, &mut payload, true, deadline)?;
    let mut ack = [0];
    transfer(&mut stream, &mut ack, false, deadline)?;
    if ack[0] != 1 {
        return Err(io::Error::other("The running instance could not accept the request"));
    }
    Ok(())
}

fn receive(stream: &mut LocalSocketStream, sender: &SyncSender<Result<Launch, String>>, ctx: &egui::Context, window: Option<isize>) -> io::Result<()> {
    stream.set_nonblocking(true)?;
    let deadline = Instant::now() + TIMEOUT;
    transfer(stream, &mut std::process::id().to_le_bytes(), true, deadline)?;
    let mut size = [0; 4];
    transfer(stream, &mut size, false, deadline)?;
    let size = u32::from_le_bytes(size) as usize;
    if size > MAX_MESSAGE {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Instance request is too large"));
    }
    let mut payload = vec![0; size];
    transfer(stream, &mut payload, false, deadline)?;
    let (file, tail, error, mut focus_error): (Option<Vec<u16>>, bool, Option<String>, Option<String>) =
        serde_json::from_slice(&payload)?;
    if let Some(window) = window {
        let window = window as *mut std::ffi::c_void;
        // The secondary grants foreground permission before sending the request.
        unsafe {
            if IsIconic(window) != 0 {
                ShowWindow(window, SW_RESTORE);
            }
            if SetForegroundWindow(window) != 0 {
                focus_error = None;
            } else if focus_error.is_none() {
                focus_error = Some("Windows did not allow the RustEd window to take focus".to_owned());
            }
        }
    }
    let launch = Launch {
        file: file.map(|wide| PathBuf::from(OsString::from_wide(&wide))),
        tail,
        error,
        focus_error,
    };
    sender.try_send(Ok(launch)).map_err(|error| io::Error::other(format!("Cannot queue launch request: {error}")))?;
    ctx.request_repaint();
    transfer(stream, &mut [1], true, deadline)
}

pub(crate) fn listen(listener: LocalSocketListener, ctx: egui::Context, window: Option<isize>) -> Receiver<Result<Launch, String>> {
    let (sender, receiver) = mpsc::sync_channel(64);
    thread::spawn(move || {
        loop {
            match listener.accept() {
                Ok(mut stream) => {
                    if let Err(error) = receive(&mut stream, &sender, &ctx, window) {
                        eprintln!("Cannot receive launch request: {error}");
                        if sender.try_send(Err(format!("Cannot receive launch request: {error}"))).is_ok() {
                            ctx.request_repaint();
                        }
                    }
                }
                Err(error) => {
                    eprintln!("Cannot accept launch requests: {error}");
                    if sender.try_send(Err(format!("Cannot accept launch requests: {error}"))).is_ok() {
                        ctx.request_repaint();
                    }
                    break;
                }
            }
        }
    });
    receiver
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forwards_launches_to_primary_and_releases_lock() {
        let directory = std::env::temp_dir().join(format!(
            "rusted-instance-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        let (lock, listener) = match Instance::acquire_at(&directory, 1).unwrap() {
            Instance::Primary { lock, listener } => (lock, listener),
            Instance::Secondary { .. } => panic!("first instance was not primary"),
        };
        let name = match Instance::acquire_at(&directory, 1).unwrap() {
            Instance::Secondary { name } => name,
            Instance::Primary { .. } => panic!("second instance was not secondary"),
        };
        let receiver = listen(listener, egui::Context::default(), None);
        forward(&name, [OsString::from("-t"), OsString::from("a b.txt")]).unwrap();
        let launch = receiver.recv_timeout(TIMEOUT).unwrap().unwrap();
        assert_eq!(launch.file, Some(std::path::absolute("a b.txt").unwrap()));
        assert!(launch.tail);
        assert!(launch.error.is_none());
        forward(&name, [OsString::from("-t")]).unwrap();
        assert_eq!(receiver.recv_timeout(TIMEOUT).unwrap().unwrap().error.as_deref(), Some("-t requires a file path"));
        forward(&name, []).unwrap();
        assert!(receiver.recv_timeout(TIMEOUT).unwrap().unwrap().file.is_none());
        drop(lock);
        let unlocked = OpenOptions::new().read(true).write(true).open(directory.join("instance-1.lock")).unwrap();
        unlocked.try_lock().unwrap();
        drop(unlocked);
        std::fs::remove_file(directory.join("instance-1.lock")).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
}
