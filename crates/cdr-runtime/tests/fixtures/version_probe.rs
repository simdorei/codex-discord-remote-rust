use std::{fs, io::Write, time::Duration};

pub fn before_version() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let exe = std::env::current_exe()?;
    let behavior = fs::read_to_string(exe.with_file_name("probe-behavior.txt")).unwrap_or_default();
    match behavior.trim() {
        "hang" => std::thread::sleep(Duration::from_mins(1)),
        "hold-pipe" => {
            let mut command = std::process::Command::new(&exe);
            command.arg("--probe-child");
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                command.creation_flags(0x0800_0000);
            }
            let child = command.spawn()?;
            fs::write(
                exe.with_file_name("probe-child.pid"),
                child.id().to_string(),
            )?;
        }
        "flood" => {
            let writer = std::thread::spawn(|| {
                let _ = std::io::stderr().write_all(&vec![b'E'; 262_144]);
            });
            let _ = std::io::stdout().write_all(&vec![b'O'; 262_144]);
            let _ = writer.join();
        }
        _ => {}
    }
    Ok(())
}
