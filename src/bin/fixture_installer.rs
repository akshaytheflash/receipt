//! Test fixture. Writes files, registry values, a service, a scheduled task,
//! a network connection and a child process so `receipt` has something to
//! record. Not part of the product.

use std::env;
use std::fs;
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::Duration;
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_WRITE};
use winreg::RegKey;

fn elevated() -> bool {
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey_with_flags("SOFTWARE", KEY_WRITE)
        .is_ok()
}

fn main() -> anyhow::Result<()> {
    let dir = env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| env::temp_dir().join("receipt-fixture"));

    fs::create_dir_all(&dir)?;
    fs::write(dir.join("alpha.txt"), "alpha")?;
    fs::write(dir.join("beta.txt"), "beta")?;
    fs::create_dir_all(dir.join("nested"))?;
    fs::write(dir.join("nested").join("gamma.txt"), "gamma")?;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (k, _) = hkcu.create_subkey(r"Software\ReceiptFixture")?;
    k.set_value("Alpha", &"one")?;
    k.set_value("Beta", &42u32)?;

    if elevated() {
        if let Ok((k2, _)) =
            RegKey::predef(HKEY_LOCAL_MACHINE).create_subkey(r"SOFTWARE\ReceiptFixture")
        {
            let _ = k2.set_value("Installed", &"yes");
        }

        let _ = Command::new("sc")
            .args(["delete", "ReceiptFixture"])
            .output();
        let _ = Command::new("sc")
            .args([
                "create",
                "ReceiptFixture",
                "binPath=",
                r"C:\Windows\System32\cmd.exe /c exit",
                "start=",
                "demand",
            ])
            .output();

        let tasks =
            PathBuf::from(env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()))
                .join("System32")
                .join("Tasks");
        let _ = fs::write(tasks.join("ReceiptFixture"), "<Task/>");
    }

    // A connection the poller can see. Bound and connected in-process so the
    // fixture works offline.
    let listener = TcpListener::bind("127.0.0.1:47821")?;
    let h = thread::spawn(move || {
        if let Ok((_s, _)) = listener.accept() {
            thread::sleep(Duration::from_millis(1500));
        }
    });
    let mut stream = TcpStream::connect("127.0.0.1:47821")?;
    let _ = stream.write_all(b"hello");
    thread::sleep(Duration::from_millis(1500));
    let _ = h.join();

    let _ = Command::new("cmd")
        .args(["/c", "ping -n 2 127.0.0.1 >nul"])
        .status();

    Ok(())
}
