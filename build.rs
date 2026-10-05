use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    println!("cargo::rerun-if-changed=icons/icon.ico");
    println!("cargo::rerun-if-env-changed=RC");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        // Leave space for macdeployqt to rewrite framework load commands.
        println!("cargo::rustc-link-arg-bin=tailer=-Wl,-headerpad_max_install_names");
        println!("cargo::rustc-link-arg-bin=tailer=-Wl,-rpath,@executable_path/../Frameworks");
    }
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    assert_eq!(
        env::var("CARGO_CFG_TARGET_ENV").as_deref(),
        Ok("msvc"),
        "Windows icon embedding requires the MSVC toolchain"
    );

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let icon = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .join("icons/icon.ico")
        .to_string_lossy()
        .replace('\\', "/");
    let resource = out_dir.join("tailer.rc");
    let compiled = out_dir.join("tailer.res");
    fs::write(&resource, format!("1 ICON \"{icon}\"\n")).unwrap();
    let status = Command::new(env::var_os("RC").unwrap_or_else(|| "rc.exe".into()))
        .arg("/nologo")
        .arg("/fo")
        .arg(&compiled)
        .arg(&resource)
        .status()
        .expect("Cannot run rc.exe; use a Visual Studio Developer shell with the Windows SDK");
    assert!(status.success(), "Windows icon resource compilation failed");
    println!("cargo::rustc-link-arg-bin=tailer={}", compiled.display());
}
