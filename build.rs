use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

fn output(command: &mut Command) -> io::Result<String> {
    let result = command.output()?;
    if !result.status.success() {
        return Err(io::Error::other(format!(
            "{command:?} failed ({}): {}",
            result.status,
            String::from_utf8_lossy(&result.stderr).trim()
        )));
    }
    String::from_utf8(result.stdout)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn git(args: &[&str]) -> io::Result<String> {
    output(Command::new("git").args(args))
}

fn main() -> io::Result<()> {
    println!("cargo:rerun-if-changed=gitversion-fast.sh");
    for name in [
        "GITHUB_HEAD_REF",
        "GITHUB_REF_NAME",
        "BUILD_SOURCEBRANCHNAME",
        "GIT_DIR",
        "GIT_COMMON_DIR",
        "GIT_WORK_TREE",
        "PATH",
    ] {
        println!("cargo:rerun-if-env-changed={name}");
    }

    let git_dir = PathBuf::from(git(&["rev-parse", "--absolute-git-dir"])?.trim());
    let common_dir = PathBuf::from(git(&["rev-parse", "--git-common-dir"])?.trim());
    for path in [
        git_dir.join("HEAD"),
        common_dir.join("refs"),
        common_dir.join("packed-refs"),
        common_dir.join("shallow"),
    ] {
        if path.exists() {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }

    let bash = if cfg!(windows) {
        let exec_path = PathBuf::from(git(&["--exec-path"])?.trim());
        let git_bash = exec_path
            .ancestors()
            .nth(3)
            .map(|root| root.join("bin").join("bash.exe"));
        match git_bash {
            Some(path) if path.is_file() => path,
            _ => PathBuf::from("bash"),
        }
    } else {
        PathBuf::from("bash")
    };
    let json = output(Command::new(bash).arg("gitversion-fast.sh"))?;
    let version: serde_json::Value = serde_json::from_str(&json)?;
    let full = version["FullSemVer"]
        .as_str()
        .filter(|value| {
            !value.is_empty()
                && value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'+'))
        })
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Invalid FullSemVer"))?;
    let component = |name| -> io::Result<u16> {
        version[name]
            .as_u64()
            .and_then(|value| u16::try_from(value).ok())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("Invalid {name}")))
    };
    let numeric = (u64::from(component("Major")?) << 48)
        | (u64::from(component("Minor")?) << 32)
        | (u64::from(component("Patch")?) << 16);
    println!("cargo:rustc-env=RUSTED_VERSION={full}");

    let icon = Path::new("src").join("assets").join("app.ico");
    println!("cargo:rerun-if-changed={}", icon.display());
    if std::env::var("CARGO_CFG_TARGET_OS").expect("Cargo sets CARGO_CFG_TARGET_OS") == "windows" {
        let icon = icon
            .to_str()
            .ok_or_else(|| io::Error::other("Icon path is not UTF-8"))?;
        winresource::WindowsResource::new()
            .set_icon(icon)
            .set("FileVersion", full)
            .set("ProductVersion", full)
            .set_version_info(winresource::VersionInfo::FILEVERSION, numeric)
            .set_version_info(winresource::VersionInfo::PRODUCTVERSION, numeric)
            .compile()?;
    }
    Ok(())
}
